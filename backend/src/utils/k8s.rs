use kube::{Client, Api, api::{PostParams, DeleteParams, PatchParams, Patch}};
use serde_json::json;
use k8s_openapi::api::core::v1::{Namespace, Service};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::networking::v1::Ingress;
use crate::utils::error::AppError;

pub struct K8sManager;

impl K8sManager {
    pub async fn get_client() -> Result<Client, AppError> {
        Client::try_default().await
            .map_err(|e| AppError::Infrastructure(format!("Failed to connect to K3s cluster: {}", e)))
    }

    pub async fn create_namespace(client: &Client, name: &str, max_mem: i32, max_storage: i32) -> Result<(), AppError> {
        let namespaces: Api<Namespace> = Api::all(client.clone());
        let ns_manifest: Namespace = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": name
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Namespace serialization failed: {}", e)))?;

        let _ = namespaces.patch(
            name,
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&ns_manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to create Namespace {}: {}", name, e)))?;

        // Apply resource limits and default quota ranges
        Self::apply_namespace_limits(client, name, max_mem, max_storage).await?;

        // Apply NetworkPolicy for namespace isolation
        Self::apply_network_policy(client, name).await?;

        Ok(())
    }

    pub async fn delete_namespace(client: &Client, name: &str) -> Result<(), AppError> {
        let namespaces: Api<Namespace> = Api::all(client.clone());
        let _ = namespaces.delete(name, &DeleteParams::default()).await;
        Ok(())
    }

    pub async fn apply_namespace_limits(client: &Client, namespace: &str, max_mem: i32, max_storage: i32) -> Result<(), AppError> {
        let quotas: Api<k8s_openapi::api::core::v1::ResourceQuota> = Api::namespaced(client.clone(), namespace);
        let limit_ranges: Api<k8s_openapi::api::core::v1::LimitRange> = Api::namespaced(client.clone(), namespace);

        if max_mem <= 0 && max_storage <= 0 {
            let _ = quotas.delete("hermes-quota", &DeleteParams::default()).await;
        } else {
            let mut hard = serde_json::Map::new();
            if max_mem > 0 {
                hard.insert("requests.memory".to_string(), json!(format!("{}Mi", max_mem)));
                hard.insert("limits.memory".to_string(), json!(format!("{}Mi", max_mem)));
            }
            if max_storage > 0 {
                hard.insert("requests.storage".to_string(), json!(format!("{}Gi", max_storage)));
            }

            if !hard.is_empty() {
                let quota_manifest: k8s_openapi::api::core::v1::ResourceQuota = serde_json::from_value(json!({
                    "apiVersion": "v1",
                    "kind": "ResourceQuota",
                    "metadata": {
                        "name": "hermes-quota",
                        "namespace": namespace
                    },
                    "spec": {
                        "hard": hard
                    }
                })).map_err(|e| AppError::Fatal(anyhow::anyhow!("ResourceQuota serialization failed: {}", e)))?;

                let _ = quotas.patch(
                    "hermes-quota",
                    &PatchParams::apply("hermes-orchestrator").force(),
                    &Patch::Apply(&quota_manifest)
                ).await
                .map_err(|e| AppError::Infrastructure(format!("Failed to apply ResourceQuota: {}", e)))?;
            } else {
                let _ = quotas.delete("hermes-quota", &DeleteParams::default()).await;
            }
        }

        // LimitRange: default limits per container as safety net
        let limit_manifest: k8s_openapi::api::core::v1::LimitRange = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "LimitRange",
            "metadata": {
                "name": "hermes-limits",
                "namespace": namespace
            },
            "spec": {
                "limits": [{
                    "type": "Container",
                    "default": {
                        "cpu": "500m",
                        "memory": "512Mi"
                    },
                    "defaultRequest": {
                        "cpu": "100m",
                        "memory": "256Mi"
                    }
                }]
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("LimitRange serialization failed: {}", e)))?;

        let _ = limit_ranges.patch(
            "hermes-limits",
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&limit_manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply LimitRange: {}", e)))?;

        Ok(())
    }

    /// Returns `(used_memory_mb, has_active_build)` for a given namespace.
    /// `used_memory_mb` is the sum of all container memory limits currently running.
    /// `has_active_build` is true when a Kaniko builder pod is found in Pending/Running phase.
    pub async fn get_namespace_resource_usage(client: &Client, namespace: &str) -> (i32, bool) {
        let pods_api: Api<k8s_openapi::api::core::v1::Pod> = Api::namespaced(client.clone(), namespace);
        let list_params = kube::api::ListParams::default();

        let pods = match pods_api.list(&list_params).await {
            Ok(p) => p,
            Err(_) => return (0, false),
        };

        let mut used_mb: i32 = 0;
        let mut has_active_build = false;

        for pod in &pods.items {
            let phase = pod.status.as_ref()
                .and_then(|s| s.phase.as_deref())
                .unwrap_or("Unknown");

            // Only count pods that are actually consuming resources
            if phase == "Succeeded" || phase == "Failed" {
                continue;
            }

            // Detect active builder pods
            let pod_name = pod.metadata.name.as_deref().unwrap_or("");
            if pod_name.starts_with("hermes-builder-") && (phase == "Pending" || phase == "Running") {
                has_active_build = true;
            }

            // Sum memory limits of all containers in this pod
            if let Some(spec) = &pod.spec {
                for container in &spec.containers {
                    if let Some(resources) = &container.resources {
                        if let Some(limits) = &resources.limits {
                            if let Some(mem_qty) = limits.get("memory") {
                                used_mb += Self::parse_memory_quantity_mb(&mem_qty.0);
                            }
                        }
                    }
                }
            }
        }

        (used_mb, has_active_build)
    }

    fn parse_memory_quantity_mb(qty_str: &str) -> i32 {
        let s = qty_str.trim();
        if s.ends_with("Gi") {
            s.trim_end_matches("Gi").parse::<i32>().unwrap_or(0) * 1024
        } else if s.ends_with("Mi") {
            s.trim_end_matches("Mi").parse::<i32>().unwrap_or(0)
        } else if s.ends_with("Ki") {
            s.trim_end_matches("Ki").parse::<i32>().unwrap_or(0) / 1024
        } else {
            s.parse::<i64>().unwrap_or(0) as i32 / 1024 / 1024
        }
    }

    /// Returns total storage used by PVCs in the namespace, in GB (as f64).
    pub async fn get_namespace_storage_usage_gb(client: &Client, namespace: &str) -> f64 {
        let pvc_api: Api<k8s_openapi::api::core::v1::PersistentVolumeClaim> =
            Api::namespaced(client.clone(), namespace);

        let pvcs = match pvc_api.list(&kube::api::ListParams::default()).await {
            Ok(p) => p,
            Err(_) => return 0.0,
        };

        let mut total_bytes: i64 = 0;
        for pvc in &pvcs.items {
            if let Some(spec) = &pvc.spec {
                if let Some(resources) = &spec.resources {
                    if let Some(requests) = &resources.requests {
                        if let Some(storage_qty) = requests.get("storage") {
                            total_bytes += Self::parse_storage_quantity_bytes(&storage_qty.0);
                        }
                    }
                }
            }
        }

        total_bytes as f64 / 1_073_741_824.0
    }

    fn parse_storage_quantity_bytes(qty_str: &str) -> i64 {
        let s = qty_str.trim();
        if s.ends_with("Ti") {
            s.trim_end_matches("Ti").parse::<i64>().unwrap_or(0) * 1_099_511_627_776
        } else if s.ends_with("Gi") {
            s.trim_end_matches("Gi").parse::<i64>().unwrap_or(0) * 1_073_741_824
        } else if s.ends_with("Mi") {
            s.trim_end_matches("Mi").parse::<i64>().unwrap_or(0) * 1_048_576
        } else if s.ends_with("Ki") {
            s.trim_end_matches("Ki").parse::<i64>().unwrap_or(0) * 1_024
        } else {
            s.parse::<i64>().unwrap_or(0)
        }
    }

    pub async fn apply_network_policy(client: &Client, namespace: &str) -> Result<(), AppError> {

        let net_policies: Api<k8s_openapi::api::networking::v1::NetworkPolicy> = Api::namespaced(client.clone(), namespace);

        let net_policy_manifest: k8s_openapi::api::networking::v1::NetworkPolicy = serde_json::from_value(json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "NetworkPolicy",
            "metadata": {
                "name": "hermes-network-isolation",
                "namespace": namespace
            },
            "spec": {
                "podSelector": {},
                "policyTypes": ["Ingress"],
                "ingress": [
                    {
                        "from": [{
                            "podSelector": {}
                        }]
                    },
                    {
                        "from": [{
                            "namespaceSelector": {
                                "matchLabels": {
                                    "kubernetes.io/metadata.name": "kube-system"
                                }
                            }
                        }]
                    }
                ]
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("NetworkPolicy serialization failed: {}", e)))?;

        let _ = net_policies.patch(
            "hermes-network-isolation",
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&net_policy_manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply NetworkPolicy: {}", e)))?;

        Ok(())
    }

    pub async fn create_secret(
        client: &Client,
        namespace: &str,
        name: &str,
        data: Vec<(String, String)>,
    ) -> Result<(), AppError> {
        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(client.clone(), namespace);
        let mut string_data = serde_json::Map::new();
        string_data.insert("HERMES_ENV_MANAGED".to_string(), json!("true"));
        for (k, v) in data {
            string_data.insert(k, json!(v));
        }

        let secret_manifest: k8s_openapi::api::core::v1::Secret = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": format!("{}-env", name),
                "namespace": namespace,
            },
            "type": "Opaque",
            "stringData": string_data
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Secret serialization failed: {}", e)))?;

        let _ = secrets.patch(
            &format!("{}-env", name),
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&secret_manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply Secret for {}: {}", name, e)))?;

        Ok(())
    }

    pub async fn deploy_app(
        client: &Client,
        namespace: &str,
        name: &str,
        image: &str,
        port: i32,
        envs: Vec<(String, String)>,
        binds: Vec<(String, String)>,
        cpu_limit: i32,
        memory_limit_mb: i64,
    ) -> Result<(), AppError> {
        let deployments: Api<Deployment> = Api::namespaced(client.clone(), namespace);

        // Apply Secret for app environment variables
        Self::create_secret(client, namespace, name, envs).await?;

        let mut volumes = Vec::new();
        let mut volume_mounts = Vec::new();
        let pvc_api: Api<k8s_openapi::api::core::v1::PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
        let pv_api: Api<k8s_openapi::api::core::v1::PersistentVolume> = Api::all(client.clone());

        for (i, (host_path, container_path)) in binds.into_iter().enumerate() {
            let vol_name = format!("volume-{}", i);
            let pvc_name = format!("{}-pvc-{}", name, i);
            let pv_name = format!("{}-pv-{}", name, i);

            let k8s_host_path = host_path.replace('\\', "/");

            // Create/patch PersistentVolume pointing to the host path
            let pv_manifest = serde_json::from_value::<k8s_openapi::api::core::v1::PersistentVolume>(json!({
                "apiVersion": "v1",
                "kind": "PersistentVolume",
                "metadata": {
                    "name": pv_name
                },
                "spec": {
                    "capacity": {
                        "storage": "1Gi"
                    },
                    "accessModes": ["ReadWriteOnce"],
                    "persistentVolumeReclaimPolicy": "Retain",
                    "hostPath": {
                        "path": k8s_host_path
                    },
                    "storageClassName": "manual",
                    "claimRef": {
                        "namespace": namespace,
                        "name": pvc_name
                    }
                }
            })).map_err(|e| AppError::Fatal(anyhow::anyhow!("PV serialization failed: {}", e)))?;

            let _ = pv_api.patch(
                &pv_name,
                &PatchParams::apply("hermes-orchestrator").force(),
                &Patch::Apply(&pv_manifest)
            ).await;

            // Create/patch PersistentVolumeClaim bound to the custom PV
            let pvc_manifest = serde_json::from_value::<k8s_openapi::api::core::v1::PersistentVolumeClaim>(json!({
                "apiVersion": "v1",
                "kind": "PersistentVolumeClaim",
                "metadata": {
                    "name": pvc_name,
                    "namespace": namespace
                },
                "spec": {
                    "accessModes": ["ReadWriteOnce"],
                    "storageClassName": "manual",
                    "volumeName": pv_name,
                    "resources": {
                        "requests": {
                            "storage": "1Gi"
                        }
                    }
                }
            })).map_err(|e| AppError::Fatal(anyhow::anyhow!("PVC serialization failed: {}", e)))?;

            let _ = pvc_api.patch(
                &pvc_name,
                &PatchParams::apply("hermes-orchestrator").force(),
                &Patch::Apply(&pvc_manifest)
            ).await;

            volumes.push(json!({
                "name": vol_name,
                "persistentVolumeClaim": {
                    "claimName": pvc_name
                }
            }));
            volume_mounts.push(json!({
                "name": vol_name,
                "mountPath": container_path
            }));
        }

        let mut resources = json!({});
        if cpu_limit > 0 || memory_limit_mb > 0 {
            let mut limits = json!({});
            if cpu_limit > 0 {
                limits["cpu"] = json!(format!("{}m", cpu_limit));
            }
            if memory_limit_mb > 0 {
                limits["memory"] = json!(format!("{}Mi", memory_limit_mb));
            }
            resources["limits"] = limits;
        }

        let deployment_manifest: Deployment = serde_json::from_value(json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": name,
                "namespace": namespace,
                "labels": {
                    "app": name
                }
            },
            "spec": {
                "replicas": 1,
                "strategy": {
                    "type": "RollingUpdate",
                    "rollingUpdate": {
                        "maxSurge": 1,
                        "maxUnavailable": 0
                    }
                },
                "selector": {
                    "matchLabels": {
                        "app": name
                    }
                },
                "template": {
                    "metadata": {
                        "labels": {
                            "app": name
                        }
                    },
                    "spec": {
                        "containers": [{
                            "name": name,
                            "image": image,
                            "ports": [{
                                "containerPort": port
                            }],
                            "envFrom": [{
                                "secretRef": {
                                    "name": format!("{}-env", name)
                                }
                            }],
                            "volumeMounts": volume_mounts,
                            "resources": resources,
                            "readinessProbe": {
                                "tcpSocket": {
                                    "port": port
                                },
                                "initialDelaySeconds": 3,
                                "periodSeconds": 5
                            }
                        }],
                        "volumes": volumes
                    }
                }
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Deployment serialization failed: {}", e)))?;

        deployments.patch(
            name,
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&deployment_manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply Deployment {}: {}", name, e)))?;

        Ok(())
    }

    pub async fn deploy_service(
        client: &Client,
        namespace: &str,
        name: &str,
        port: i32,
    ) -> Result<(), AppError> {
        let services: Api<Service> = Api::namespaced(client.clone(), namespace);

        let service_manifest: Service = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": {
                "name": name,
                "namespace": namespace,
                "labels": {
                    "app": name
                }
            },
            "spec": {
                "ports": [{
                    "port": port,
                    "targetPort": port
                }],
                "selector": {
                    "app": name
                }
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Service serialization failed: {}", e)))?;

        services.patch(
            name,
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&service_manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply Service {}: {}", name, e)))?;

        Ok(())
    }

    pub async fn deploy_loadbalancer_service(
        client: &Client,
        namespace: &str,
        name: &str,
        label_selector: &str,
        internal_port: i32,
        external_port: i32,
        protocol: &str,
    ) -> Result<(), AppError> {
        let services: Api<Service> = Api::namespaced(client.clone(), namespace);

        let service_manifest: Service = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": {
                "name": name,
                "namespace": namespace,
                "labels": {
                    "app": label_selector
                }
            },
            "spec": {
                "type": "LoadBalancer",
                "ports": [{
                    "name": format!("{}-port", protocol.to_lowercase()),
                    "port": external_port,
                    "targetPort": internal_port,
                    "protocol": protocol
                }],
                "selector": {
                    "app": label_selector
                }
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("LoadBalancer Service serialization failed: {}", e)))?;

        services.patch(
            name,
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&service_manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply LoadBalancer Service {}: {}", name, e)))?;

        Ok(())
    }

    pub async fn delete_loadbalancer_service(
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> Result<(), AppError> {
        let services: Api<Service> = Api::namespaced(client.clone(), namespace);
        let _ = services.delete(name, &DeleteParams::default()).await;
        Ok(())
    }

    pub async fn deploy_ingress(
        client: &Client,
        namespace: &str,
        name: &str,
        host: &str,
        service_name: &str,
        service_port: i32,
    ) -> Result<(), AppError> {
        let ingresses: Api<Ingress> = Api::namespaced(client.clone(), namespace);

        let mut annotations = serde_json::Map::new();
        annotations.insert("ingress.kubernetes.io/ssl-redirect".to_string(), json!("false"));

        let mut tls = Vec::new();
        if let Ok(issuer) = std::env::var("HERMES_SSL_ISSUER") {
            if !issuer.is_empty() {
                annotations.insert("cert-manager.io/cluster-issuer".to_string(), json!(issuer));
                tls.push(json!({
                    "hosts": [host],
                    "secretName": format!("tls-{}", name)
                }));
            }
        }

        let ingress_manifest: Ingress = serde_json::from_value(json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "Ingress",
            "metadata": {
                "name": name,
                "namespace": namespace,
                "annotations": annotations
            },
            "spec": {
                "tls": if tls.is_empty() { json!(null) } else { json!(tls) },
                "rules": [{
                    "host": host,
                    "http": {
                        "paths": [{
                            "path": "/",
                            "pathType": "Prefix",
                            "backend": {
                                "service": {
                                    "name": service_name,
                                    "port": {
                                        "number": service_port
                                    }
                                }
                            }
                        }]
                    }
                }]
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Ingress serialization failed: {}", e)))?;

        ingresses.patch(
            name,
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&ingress_manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply Ingress {}: {}", name, e)))?;

        Ok(())
    }

    pub async fn delete_ingress(
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> Result<(), AppError> {
        let ingresses: Api<Ingress> = Api::namespaced(client.clone(), namespace);
        let _ = ingresses.delete(name, &DeleteParams::default()).await;
        Ok(())
    }

    pub async fn delete_app(
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> Result<(), AppError> {
        let deployments: Api<Deployment> = Api::namespaced(client.clone(), namespace);
        let _ = deployments.delete(name, &DeleteParams::default()).await;

        let services: Api<Service> = Api::namespaced(client.clone(), namespace);
        let _ = services.delete(name, &DeleteParams::default()).await;
        let _ = services.delete(&format!("{}-external", name), &DeleteParams::default()).await;
        if let Ok(svc_list) = services.list(&kube::api::ListParams::default()).await {
            for svc in svc_list.items {
                if let Some(ref svc_name) = svc.metadata.name {
                    if svc_name.starts_with(&format!("{}-port-", name)) {
                        let _ = services.delete(svc_name, &DeleteParams::default()).await;
                    }
                }
            }
        }

        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(client.clone(), namespace);
        let _ = secrets.delete(&format!("{}-env", name), &DeleteParams::default()).await;

        // Clean up PVCs associated with the application
        let pvc_api: Api<k8s_openapi::api::core::v1::PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
        if let Ok(pvc_list) = pvc_api.list(&kube::api::ListParams::default()).await {
            for pvc in pvc_list.items {
                if let Some(ref pvc_name) = pvc.metadata.name {
                    if pvc_name.starts_with(&format!("{}-pvc-", name)) {
                        let _ = pvc_api.delete(pvc_name, &DeleteParams::default()).await;
                    }
                }
            }
        }

        // Clean up PVs associated with the application
        let pv_api: Api<k8s_openapi::api::core::v1::PersistentVolume> = Api::all(client.clone());
        if let Ok(pv_list) = pv_api.list(&kube::api::ListParams::default()).await {
            for pv in pv_list.items {
                if let Some(ref pv_name) = pv.metadata.name {
                    if pv_name.starts_with(&format!("{}-pv-", name)) {
                        let _ = pv_api.delete(pv_name, &DeleteParams::default()).await;
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn deploy_database(
        client: &Client,
        namespace: &str,
        name: &str,
        image: &str,
        envs: Vec<(String, String)>,
        port: i32,
        cpu_limit: i32,
        memory_limit_mb: i64,
    ) -> Result<(), AppError> {
        let statefulsets: Api<k8s_openapi::api::apps::v1::StatefulSet> = Api::namespaced(client.clone(), namespace);

        // Apply Secret for database environment variables
        Self::create_secret(client, namespace, name, envs).await?;

        // Determine mountPath dynamically based on the database image type
        let image_lower = image.to_lowercase();
        let mount_path = if image_lower.contains("postgres") {
            "/var/lib/postgresql"
        } else if image_lower.contains("mysql") {
            "/var/lib/mysql"
        } else if image_lower.contains("mongo") {
            "/data/db"
        } else {
            "/data" // redis and default
        };

        let mut resources = json!({});
        if cpu_limit > 0 || memory_limit_mb > 0 {
            let mut limits = json!({});
            if cpu_limit > 0 {
                limits["cpu"] = json!(format!("{}m", cpu_limit));
            }
            if memory_limit_mb > 0 {
                limits["memory"] = json!(format!("{}Mi", memory_limit_mb));
            }
            resources["limits"] = limits;
        }

        let statefulset_manifest: k8s_openapi::api::apps::v1::StatefulSet = serde_json::from_value(json!({
            "apiVersion": "apps/v1",
            "kind": "StatefulSet",
            "metadata": {
                "name": name,
                "namespace": namespace,
                "labels": {
                    "app": name
                }
            },
            "spec": {
                "serviceName": name,
                "replicas": 1,
                "selector": {
                    "matchLabels": {
                        "app": name
                    }
                },
                "template": {
                    "metadata": {
                        "labels": {
                            "app": name
                        }
                    },
                    "spec": {
                        "containers": [{
                            "name": name,
                            "image": image,
                            "ports": [{
                                "containerPort": port
                            }],
                            "envFrom": [{
                                "secretRef": {
                                    "name": format!("{}-env", name)
                                }
                            }],
                            "volumeMounts": [{
                                "name": "db-storage",
                                "mountPath": mount_path
                            }],
                            "resources": resources,
                            "readinessProbe": {
                                "tcpSocket": {
                                    "port": port
                                },
                                "initialDelaySeconds": 5,
                                "periodSeconds": 2
                            }
                        }]
                    }
                },
                "volumeClaimTemplates": [{
                    "metadata": {
                        "name": "db-storage"
                    },
                    "spec": {
                        "accessModes": ["ReadWriteOnce"],
                        "resources": {
                            "requests": {
                                "storage": "1Gi"
                            }
                        }
                    }
                }]
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("DB StatefulSet serialization failed: {}", e)))?;

        statefulsets.patch(
            name,
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&statefulset_manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply DB StatefulSet {}: {}", name, e)))?;

        Self::deploy_service(client, namespace, name, port).await?;
        Ok(())
    }

    pub async fn delete_database(
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> Result<(), AppError> {
        let statefulsets: Api<k8s_openapi::api::apps::v1::StatefulSet> = Api::namespaced(client.clone(), namespace);
        let _ = statefulsets.delete(name, &DeleteParams::default()).await;

        let services: Api<Service> = Api::namespaced(client.clone(), namespace);
        let _ = services.delete(name, &DeleteParams::default()).await;
        let _ = services.delete(&format!("{}-external", name), &DeleteParams::default()).await;

        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(client.clone(), namespace);
        let _ = secrets.delete(&format!("{}-env", name), &DeleteParams::default()).await;

        // Clean up database PVC
        let pvc_api: Api<k8s_openapi::api::core::v1::PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
        let _ = pvc_api.delete(&format!("db-storage-{}-0", name), &DeleteParams::default()).await;

        Ok(())
    }

    pub async fn deploy_knative_service(
        client: &Client,
        namespace: &str,
        name: &str,
        image: &str,
        envs: Vec<(String, String)>,
        min_scale: i32,
        max_scale: i32,
        target_concurrency: i32,
        memory_limit_mb: Option<i32>,
        // When set, stamps a changing annotation onto the revision template so Knative
        // creates a fresh revision even if the image is unchanged — used for env-only
        // reloads (Knative ignores changes to the referenced envFrom secret alone).
        reload_token: Option<String>,
    ) -> Result<(), AppError> {
        // Apply Secret for Knative service environment variables
        Self::create_secret(client, namespace, name, envs).await?;

        let gvk = kube::api::GroupVersionKind::gvk("serving.knative.dev", "v1", "Service");
        let api_resource = kube::api::ApiResource::from_gvk_with_plural(&gvk, "services");
        let dynamic_api = kube::Api::<kube::core::DynamicObject>::namespaced_with(
            client.clone(),
            namespace,
            &api_resource
        );

        let mut container = json!({
            "image": image,
            "envFrom": [{
                "secretRef": {
                    "name": format!("{}-env", name)
                }
            }]
        });

        if let Some(mem_limit) = memory_limit_mb {
            if mem_limit > 0 {
                container["resources"] = json!({
                    "requests": {
                        "memory": format!("{}Mi", mem_limit)
                    },
                    "limits": {
                        "memory": format!("{}Mi", mem_limit)
                    }
                });
            }
        }

        let mut template_annotations = json!({
            "autoscaling.knative.dev/min-scale": min_scale.to_string(),
            "autoscaling.knative.dev/max-scale": max_scale.to_string(),
            "autoscaling.knative.dev/target": target_concurrency.to_string()
        });
        if let Some(token) = reload_token {
            template_annotations["hermes.dev/env-reload"] = json!(token);
        }

        let manifest: kube::core::DynamicObject = serde_json::from_value(json!({
            "apiVersion": "serving.knative.dev/v1",
            "kind": "Service",
            "metadata": {
                "name": name,
                "namespace": namespace
            },
            "spec": {
                "template": {
                    "metadata": {
                        "annotations": template_annotations
                    },
                    "spec": {
                        "containers": [container]
                    }
                }
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Knative Service serialization failed: {}", e)))?;

        let _ = dynamic_api.patch(
            name,
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply Knative Service {}: {}", name, e)))?;

        Ok(())
    }

    pub async fn delete_knative_service(
        client: &Client,
        namespace: &str,
        name: &str,
    ) -> Result<(), AppError> {
        let gvk = kube::api::GroupVersionKind::gvk("serving.knative.dev", "v1", "Service");
        let api_resource = kube::api::ApiResource::from_gvk_with_plural(&gvk, "services");
        let dynamic_api = kube::Api::<kube::core::DynamicObject>::namespaced_with(
            client.clone(),
            namespace,
            &api_resource
        );
        let _ = dynamic_api.delete(name, &DeleteParams::default()).await;

        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(client.clone(), namespace);
        let _ = secrets.delete(&format!("{}-env", name), &DeleteParams::default()).await;

        Ok(())
    }

    pub async fn scale_deployment(
        client: &Client,
        namespace: &str,
        name: &str,
        replicas: i32,
    ) -> Result<(), AppError> {
        let deployments: Api<Deployment> = Api::namespaced(client.clone(), namespace);
        let patch = json!({
            "spec": {
                "replicas": replicas
            }
        });
        let _ = deployments.patch(
            name,
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&patch)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to scale Deployment {}: {}", name, e)))?;
        Ok(())
    }

    pub async fn run_job_and_get_logs(
        client: &Client,
        namespace: &str,
        name: &str,
        image: &str,
        envs: Vec<(String, String)>,
        command: &str,
    ) -> Result<(String, i32), AppError> {
        struct SecretCleanupGuard {
            client: Client,
            namespace: String,
            name: String,
        }

        impl Drop for SecretCleanupGuard {
            fn drop(&mut self) {
                let client = self.client.clone();
                let namespace = self.namespace.clone();
                let name = self.name.clone();
                tokio::spawn(async move {
                    let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(client, &namespace);
                    let _ = secrets.delete(&format!("{}-env", name), &DeleteParams::default()).await;
                });
            }
        }

        let jobs: Api<k8s_openapi::api::batch::v1::Job> = Api::namespaced(client.clone(), namespace);

        // Clean up any pre-existing job with the same name to avoid 409 conflict errors
        if jobs.get(name).await.is_ok() {
            let delete_params = DeleteParams {
                propagation_policy: Some(kube::api::PropagationPolicy::Background),
                ..Default::default()
            };
            let _ = jobs.delete(name, &delete_params).await;
            // Wait up to 5 seconds for it to be deleted by k8s
            for _ in 0..10 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if jobs.get(name).await.is_err() {
                    break;
                }
            }
        }

        // Apply Secret for job environment variables
        Self::create_secret(client, namespace, name, envs).await?;

        // Instantiate guard to ensure the secret is deleted on all exit paths
        let _cleanup_guard = SecretCleanupGuard {
            client: client.clone(),
            namespace: namespace.to_string(),
            name: name.to_string(),
        };

        let job_manifest: k8s_openapi::api::batch::v1::Job = serde_json::from_value(json!({
            "apiVersion": "batch/v1",
            "kind": "Job",
            "metadata": {
                "name": name,
                "namespace": namespace,
            },
            "spec": {
                "backoffLimit": 0,
                "ttlSecondsAfterFinished": 60,
                "activeDeadlineSeconds": 300,
                "template": {
                    "spec": {
                        "restartPolicy": "Never",
                        "containers": [{
                            "name": name,
                            "image": image,
                            "imagePullPolicy": "IfNotPresent",
                            "command": ["/bin/sh", "-c", command],
                            "resources": {
                                "requests": {
                                    "memory": "32Mi",
                                    "cpu": "50m"
                                },
                                "limits": {
                                    "memory": "64Mi",
                                    "cpu": "200m"
                                }
                            },
                            "envFrom": [{
                                "secretRef": {
                                    "name": format!("{}-env", name)
                                }
                            }]
                        }]
                    }
                }
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Job serialization failed: {}", e)))?;

        let _ = jobs.create(&PostParams::default(), &job_manifest).await
            .map_err(|e| AppError::Infrastructure(format!("Failed to create Job {}: {}", name, e)))?;

        // Wait for job to finish
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            if let Ok(job_status) = jobs.get(name).await {
                if let Some(status) = job_status.status {
                    if let Some(succeeded) = status.succeeded {
                        if succeeded > 0 {
                            break;
                        }
                    }
                    if let Some(failed) = status.failed {
                        if failed > 0 {
                            break;
                        }
                    }
                }
            } else {
                return Err(AppError::Infrastructure(format!("Job {} disappeared", name)));
            }
        }

        // Get pod and its logs
        let pods_api: Api<k8s_openapi::api::core::v1::Pod> = Api::namespaced(client.clone(), namespace);
        let lp = kube::api::ListParams::default().labels(&format!("job-name={}", name));
        let pod_list = pods_api.list(&lp).await
            .map_err(|e| AppError::Infrastructure(format!("Failed to list pods for job: {}", e)))?;
        let pod = pod_list.items.first()
            .ok_or_else(|| AppError::NotFound(format!("No pod found for job {}", name)))?;
        let pod_name = pod.metadata.name.as_ref()
            .ok_or_else(|| AppError::NotFound(format!("No pod name found for job {}", name)))?;

        let log_params = kube::api::LogParams {
            follow: false,
            ..Default::default()
        };
        let logs_str = match pods_api.logs(pod_name, &log_params).await {
            Ok(l) => l,
            Err(e) => {
                println!("[K8sManager] Error getting logs for pod {}: {:?}", pod_name, e);
                format!("Failed to retrieve logs from Kubernetes: {:?}", e)
            }
        };

        let exit_code = pod.status.as_ref()
            .and_then(|s| s.container_statuses.as_ref())
            .and_then(|statuses| statuses.first())
            .and_then(|status| status.state.as_ref())
            .and_then(|state| state.terminated.as_ref())
            .map(|term| term.exit_code)
            .unwrap_or(0);

        // Cleanup job
        let delete_params = DeleteParams {
            propagation_policy: Some(kube::api::PropagationPolicy::Background),
            ..Default::default()
        };
        let _ = jobs.delete(name, &delete_params).await;

        Ok((logs_str, exit_code))
    }
}
