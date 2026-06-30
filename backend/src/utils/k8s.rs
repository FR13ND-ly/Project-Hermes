use kube::{Client, Api, api::{PostParams, DeleteParams, PatchParams, Patch}};
use serde_json::json;
use k8s_openapi::api::core::v1::{Namespace, Service, Pod};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::networking::v1::Ingress;
use crate::utils::error::AppError;

pub struct K8sManager;

impl K8sManager {
    pub async fn get_client() -> Result<Client, AppError> {
        Client::try_default().await
            .map_err(|e| AppError::Infrastructure(format!("Failed to connect to K3s cluster: {}", e)))
    }

    /// The namespace the control plane itself runs in — where the `hermes-backups`
    /// and object-storage PVCs live. Read from the in-cluster service-account token
    /// when present, overridable via `HERMES_SYSTEM_NAMESPACE`, defaulting to
    /// `hermes-system`.
    pub fn system_namespace() -> String {
        std::env::var("HERMES_SYSTEM_NAMESPACE")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::fs::read_to_string("/var/run/secrets/kubernetes.io/serviceaccount/namespace")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| "hermes-system".to_string())
    }

    /// Resolve the first pod name matching label `app={app_label}` in a namespace.
    /// Used to target a workload's pod for `exec` (StatefulSet/Deployment-agnostic).
    pub async fn pod_name_for_app(client: &Client, namespace: &str, app_label: &str) -> Result<String, AppError> {
        let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
        let lp = kube::api::ListParams::default().labels(&format!("app={}", app_label));
        let list = pods.list(&lp).await
            .map_err(|e| AppError::Infrastructure(format!("Failed to list pods: {}", e)))?;
        let mut pods_items = list.items;
        let pod = pods_items.iter().find(|p| p.metadata.deletion_timestamp.is_none())
            .or_else(|| pods_items.first());
        let name = pod.and_then(|p| p.metadata.name.clone())
            .ok_or_else(|| AppError::Infrastructure(format!("No running pod found for '{}'.", app_label)))?;
        Ok(name)
    }

    /// Delete a pod (best-effort). A controller (StatefulSet/Deployment) recreates
    /// it, so this is used to force a restart that picks up updated Secret values
    /// (e.g. Redis re-reading its requirepass).
    pub async fn delete_pod(client: &Client, namespace: &str, pod: &str) -> Result<(), AppError> {
        let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
        let _ = pods.delete(pod, &DeleteParams::default()).await;
        Ok(())
    }

    /// Exec a command inside a pod's first container and return its stdout. Fails
    /// (Err) when the command's exit status is not Success. Requires the kube `ws`
    /// feature. Used for live credential rotation (ALTER USER, etc.).
    #[tracing::instrument(skip_all, fields(namespace = %namespace, pod = %pod), err)]
    pub async fn exec_in_pod(
        client: &Client,
        namespace: &str,
        pod: &str,
        command: Vec<String>,
    ) -> Result<String, AppError> {
        use kube::api::AttachParams;
        use tokio::io::AsyncReadExt;

        let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
        let ap = AttachParams::default().stdout(true).stderr(true).stdin(false);

        let mut attached = pods.exec(pod, command, &ap).await
            .map_err(|e| AppError::Infrastructure(format!("Pod exec failed to start: {}", e)))?;

        let mut stdout = String::new();
        if let Some(mut s) = attached.stdout() {
            let _ = s.read_to_string(&mut stdout).await;
        }
        let mut stderr = String::new();
        if let Some(mut s) = attached.stderr() {
            let _ = s.read_to_string(&mut stderr).await;
        }

        // The k8s error channel carries the exit status (status == "Success" on exit 0).
        let status = match attached.take_status() {
            Some(fut) => fut.await,
            None => None,
        };
        let _ = attached.join().await;

        let failed = status
            .as_ref()
            .and_then(|s| s.status.as_deref())
            .map(|s| s != "Success")
            .unwrap_or(false);

        if failed {
            let detail = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
            return Err(AppError::Infrastructure(format!("Command in pod '{}' failed: {}", pod, detail)));
        }

        Ok(if stdout.trim().is_empty() { stderr } else { stdout })
    }

    #[tracing::instrument(skip_all, fields(namespace = %name), err)]
    pub async fn create_namespace(client: &Client, name: &str, max_mem: i32, max_storage: i32, max_cpu: i32) -> Result<(), AppError> {
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
        Self::apply_namespace_limits(client, name, max_mem, max_storage, max_cpu).await?;

        // Apply NetworkPolicy for namespace isolation
        Self::apply_network_policy(client, name).await?;

        // kpack builds reference a `hermes-kpack` ServiceAccount in the workspace
        // namespace (see builder::run_kpack_build). Ensure it exists so the first build
        // in a fresh workspace doesn't fail. The in-cluster registry is anonymous-push,
        // so no registry secret is attached; private-git creds are a future addition.
        Self::ensure_kpack_service_account(client, name).await?;

        Ok(())
    }

    /// Ensure the `hermes-kpack` ServiceAccount exists in a workspace namespace. kpack
    /// `Image` resources run their build pods under it; absent it, kpack rejects the
    /// build. Idempotent (server-side apply).
    pub async fn ensure_kpack_service_account(client: &Client, namespace: &str) -> Result<(), AppError> {
        let sas: Api<k8s_openapi::api::core::v1::ServiceAccount> = Api::namespaced(client.clone(), namespace);
        let sa_manifest: k8s_openapi::api::core::v1::ServiceAccount = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "ServiceAccount",
            "metadata": {
                "name": "hermes-kpack",
                "namespace": namespace,
                "labels": { "app": "hermes", "managed-by": "hermes-orchestrator" }
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("kpack ServiceAccount serialization failed: {}", e)))?;

        let _ = sas.patch(
            "hermes-kpack",
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&sa_manifest)
        ).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply hermes-kpack ServiceAccount in {}: {}", namespace, e)))?;

        Ok(())
    }

    pub async fn delete_namespace(client: &Client, name: &str) -> Result<(), AppError> {
        let namespaces: Api<Namespace> = Api::all(client.clone());
        let _ = namespaces.delete(name, &DeleteParams::default()).await;
        Ok(())
    }

    pub async fn apply_namespace_limits(client: &Client, namespace: &str, max_mem: i32, max_storage: i32, max_cpu: i32) -> Result<(), AppError> {
        let quotas: Api<k8s_openapi::api::core::v1::ResourceQuota> = Api::namespaced(client.clone(), namespace);
        let limit_ranges: Api<k8s_openapi::api::core::v1::LimitRange> = Api::namespaced(client.clone(), namespace);

        // No workspace quota set (the default) → impose nothing at all. Both the
        // quota and the LimitRange are removed, so containers run unlimited unless a
        // per-resource limit is set on the workload itself.
        if max_mem <= 0 && max_storage <= 0 && max_cpu <= 0 {
            let _ = quotas.delete("hermes-quota", &DeleteParams::default()).await;
            let _ = limit_ranges.delete("hermes-limits", &DeleteParams::default()).await;
            return Ok(());
        } else {
            // Track only `requests.*`, never `limits.*`. A quota that constrains
            // `limits.memory`/`limits.cpu` forces *every* pod in the namespace to
            // declare those limits or be rejected — which would break uncapped
            // workloads (apps/databases with no limit, and Knative pods that don't
            // set a CPU limit). The LimitRange below supplies a defaultRequest so
            // every pod has a request counted here, while staying uncapped unless it
            // sets its own limit (matching the platform's "never cap unless asked"
            // design). Guaranteed-QoS workloads mirror limits into requests, so their
            // full limit still counts against the workspace cap.
            let mut hard = serde_json::Map::new();
            if max_mem > 0 {
                hard.insert("requests.memory".to_string(), json!(format!("{}Mi", max_mem)));
            }
            if max_storage > 0 {
                hard.insert("requests.storage".to_string(), json!(format!("{}Gi", max_storage)));
            }
            if max_cpu > 0 {
                hard.insert("requests.cpu".to_string(), json!(format!("{}m", max_cpu)));
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

        // A workspace quota is in effect. Provide only a small defaultRequest so pods
        // that don't declare requests can still be scheduled/counted against the quota.
        // Deliberately NO `default` limit — a container is never capped unless its own
        // limit is set explicitly.
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
                    "defaultRequest": {
                        "cpu": "50m",
                        "memory": "64Mi"
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
                                used_mb += crate::utils::quantity::parse_memory_mib(&mem_qty.0) as i32;
                            }
                        }
                    }
                }
            }
        }

        (used_mb, has_active_build)
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
                            total_bytes += crate::utils::quantity::parse_memory_bytes(&storage_qty.0) as i64;
                        }
                    }
                }
            }
        }

        total_bytes as f64 / 1_073_741_824.0
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
                    },
                    {
                        // Allow the in-cluster control plane (hermes-system) to reach
                        // workspace DBs directly for backup/restore/query/rotation —
                        // no more pod exec.
                        "from": [{
                            "namespaceSelector": {
                                "matchLabels": {
                                    "kubernetes.io/metadata.name": "hermes-system"
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

    #[tracing::instrument(skip_all, fields(namespace = %namespace, name = %name, image = %image), err)]
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
        replicas_min: i32,
        replicas_max: i32,
        autoscale_cpu_percent: i32,
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

            if let Err(e) = pv_api.patch(
                &pv_name,
                &PatchParams::apply("hermes-orchestrator").force(),
                &Patch::Apply(&pv_manifest)
            ).await {
                // Swallowing this used to silently leave the PVC Pending forever (e.g. when
                // the control-plane RBAC lacked `persistentvolumes`). Surface it instead.
                tracing::error!(pv = %pv_name, "Failed to create app-volume PersistentVolume: {}", e);
            }

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

            if let Err(e) = pvc_api.patch(
                &pvc_name,
                &PatchParams::apply("hermes-orchestrator").force(),
                &Patch::Apply(&pvc_manifest)
            ).await {
                tracing::error!(pvc = %pvc_name, "Failed to create app-volume PersistentVolumeClaim: {}", e);
            }

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
            // Mirror limits into requests → Guaranteed QoS (predictable scheduling,
            // not evicted under node pressure). No limit configured = no request, so
            // unconfigured resources stay unlimited (LimitRange is the safety net).
            resources["requests"] = limits.clone();
            resources["limits"] = limits;
        }

        // When an HPA is active (max > min) the Deployment must NOT own `replicas` —
        // otherwise every (re)apply resets it to min and fights the autoscaler. Omitting
        // the field (Null → None → not serialized under SSA) lets the HPA own the count.
        let min_r = replicas_min.max(1);
        let max_r = replicas_max.max(min_r);
        let replicas_field = if max_r > min_r { serde_json::Value::Null } else { json!(min_r) };

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
                "replicas": replicas_field,
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

        // Reconcile the autoscaler: HPA when a real range is set, otherwise none.
        Self::apply_hpa(client, namespace, name, replicas_min.max(1), replicas_max.max(replicas_min.max(1)), autoscale_cpu_percent).await;

        Ok(())
    }

    /// Reconciles a CPU-target HorizontalPodAutoscaler for a deployment. When
    /// `max <= min` no autoscaling is wanted, so any existing HPA is removed (it
    /// would otherwise pin the replica count). Best-effort: HPA needs metrics-server
    /// in the cluster, so failures are logged but never block a deploy.
    async fn apply_hpa(client: &Client, namespace: &str, name: &str, min: i32, max: i32, cpu_target: i32) {
        let hpas: Api<k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler> =
            Api::namespaced(client.clone(), namespace);

        if max <= min {
            let _ = hpas.delete(name, &DeleteParams::default()).await;
            return;
        }

        let manifest: k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler =
            match serde_json::from_value(json!({
                "apiVersion": "autoscaling/v2",
                "kind": "HorizontalPodAutoscaler",
                "metadata": { "name": name, "namespace": namespace },
                "spec": {
                    "scaleTargetRef": { "apiVersion": "apps/v1", "kind": "Deployment", "name": name },
                    "minReplicas": min,
                    "maxReplicas": max,
                    "metrics": [{
                        "type": "Resource",
                        "resource": {
                            "name": "cpu",
                            "target": { "type": "Utilization", "averageUtilization": cpu_target.clamp(1, 100) }
                        }
                    }]
                }
            })) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(%name, "HPA manifest build failed: {}", e);
                    return;
                }
            };

        if let Err(e) = hpas
            .patch(name, &PatchParams::apply("hermes-orchestrator").force(), &Patch::Apply(&manifest))
            .await
        {
            tracing::warn!(%name, "Failed to apply HPA (is metrics-server installed?): {}", e);
        }
    }

    pub async fn deploy_service(
        client: &Client,
        namespace: &str,
        name: &str,
        port: i32,
        network_alias: Option<&str>,
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

        // Stable alias Service so OTHER apps can reach this one at an address that
        // survives recreation. Uses the app's custom network_alias if set, else the
        // auto name (container_name without the per-instance hash). Non-fatal.
        let alias_name: Option<&str> = match network_alias {
            Some(a) if !a.trim().is_empty() => Some(a),
            _ => name.rsplit_once('-').map(|(p, _)| p),
        };
        if let Some(stable_name) = alias_name {
            if !stable_name.is_empty() && stable_name != name {
                let alias: Service = serde_json::from_value(json!({
                    "apiVersion": "v1",
                    "kind": "Service",
                    "metadata": {
                        "name": stable_name,
                        "namespace": namespace,
                        "labels": { "app": stable_name, "hermes-alias-for": name }
                    },
                    "spec": {
                        "ports": [{ "port": port, "targetPort": port }],
                        "selector": { "app": name }
                    }
                })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Alias Service serialization failed: {}", e)))?;

                if let Err(e) = services.patch(
                    stable_name,
                    &PatchParams::apply("hermes-orchestrator").force(),
                    &Patch::Apply(&alias)
                ).await {
                    tracing::warn!(%stable_name, %name, "Failed to apply stable alias Service: {}", e);
                }
            }
        }

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

    #[tracing::instrument(skip_all, fields(namespace = %namespace, name = %name, host = %host), err)]
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

        // Remove the autoscaler (if any) so it doesn't outlive the deployment.
        let hpas: Api<k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler> =
            Api::namespaced(client.clone(), namespace);
        let _ = hpas.delete(name, &DeleteParams::default()).await;

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

    #[tracing::instrument(skip_all, fields(namespace = %namespace, name = %name, image = %image), err)]
    pub async fn deploy_database(
        client: &Client,
        namespace: &str,
        name: &str,
        image: &str,
        envs: Vec<(String, String)>,
        port: i32,
        cpu_limit: i32,
        memory_limit_mb: i64,
        storage_gb: i32,
    ) -> Result<(), AppError> {
        let statefulsets: Api<k8s_openapi::api::apps::v1::StatefulSet> = Api::namespaced(client.clone(), namespace);

        let image_lower = image.to_lowercase();
        let is_postgres = image_lower.contains("postgres");
        let is_redis = image_lower.contains("redis");
        let is_mongo = image_lower.contains("mongo");

        // Per-engine metrics exporter (sidecar) so DB-internal metrics
        // (connections, cache-hit rate) become real in Prometheus. Connection
        // creds go into the env Secret, never the pod spec.
        struct Exporter {
            image: &'static str,
            port: i32,
            env: Vec<(String, String)>,
            args: Option<Vec<String>>,
        }
        let enc = |s: &str| {
            percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
        };
        let get = |k: &str| envs.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());

        let exporter: Option<Exporter> = if is_postgres {
            let user = get("POSTGRES_USER").unwrap_or_else(|| "postgres".to_string());
            let pass = get("POSTGRES_PASSWORD").unwrap_or_default();
            let db = get("POSTGRES_DB").unwrap_or_else(|| user.clone());
            let dsn = format!(
                "postgresql://{}:{}@localhost:{}/{}?sslmode=disable",
                enc(&user), enc(&pass), port, enc(&db)
            );
            Some(Exporter {
                image: "quay.io/prometheuscommunity/postgres-exporter:v0.15.0",
                port: 9187,
                env: vec![("DATA_SOURCE_NAME".to_string(), dsn)],
                args: None,
            })
        } else if is_redis {
            // Hermes redis runs without auth; the exporter only needs the address.
            Some(Exporter {
                image: "oliver006/redis_exporter:v1.62.0",
                port: 9121,
                env: vec![("REDIS_ADDR".to_string(), format!("redis://localhost:{}", port))],
                args: None,
            })
        } else if is_mongo {
            let user = get("MONGO_INITDB_ROOT_USERNAME").unwrap_or_default();
            let pass = get("MONGO_INITDB_ROOT_PASSWORD").unwrap_or_default();
            let uri = if user.is_empty() {
                format!("mongodb://localhost:{}", port)
            } else {
                format!("mongodb://{}:{}@localhost:{}/?authSource=admin", enc(&user), enc(&pass), port)
            };
            Some(Exporter {
                image: "percona/mongodb_exporter:0.40.0",
                port: 9216,
                env: vec![("MONGODB_URI".to_string(), uri)],
                args: Some(vec!["--collect-all".to_string(), "--compatible-mode".to_string()]),
            })
        } else {
            None
        };

        let mut secret_envs = envs.clone();
        if let Some(ref ex) = exporter {
            secret_envs.extend(ex.env.clone());
        }

        // Apply Secret for database environment variables (+ exporter creds).
        Self::create_secret(client, namespace, name, secret_envs).await?;

        // Determine mountPath dynamically based on the database image type
        let mount_path = if is_postgres {
            "/var/lib/postgresql"
        } else if image_lower.contains("mysql") {
            "/var/lib/mysql"
        } else if is_mongo {
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
            // Mirror limits into requests → Guaranteed QoS (predictable scheduling,
            // not evicted under node pressure). No limit configured = no request, so
            // unconfigured resources stay unlimited (LimitRange is the safety net).
            resources["requests"] = limits.clone();
            resources["limits"] = limits;
        }

        // Build the container list (DB + optional exporter sidecar) and pod
        // annotations so Prometheus' pod-scrape job discovers the exporter.
        let mut main_container = json!({
            "name": name,
            "image": image,
            "ports": [{ "containerPort": port }],
            "envFrom": [{ "secretRef": { "name": format!("{}-env", name) } }],
            "volumeMounts": [{ "name": "db-storage", "mountPath": mount_path }],
            "resources": resources,
            "readinessProbe": {
                "tcpSocket": { "port": port },
                "initialDelaySeconds": 5,
                "periodSeconds": 2
            }
        });
        if is_redis {
            // Redis takes its password as a server arg read from the env Secret
            // (kept out of the pod spec). Empty/unset REDIS_PASSWORD = authless.
            main_container["command"] = json!([
                "sh", "-c", "exec redis-server --requirepass \"$REDIS_PASSWORD\""
            ]);
        }
        let mut containers = vec![main_container];
        let mut pod_annotations = json!({});
        if let Some(ref ex) = exporter {
            let mut exporter_container = json!({
                "name": "metrics-exporter",
                "image": ex.image,
                "ports": [{ "containerPort": ex.port, "name": "metrics" }],
                "envFrom": [{ "secretRef": { "name": format!("{}-env", name) } }],
                "resources": { "limits": { "cpu": "100m", "memory": "128Mi" } }
            });
            if let Some(ref args) = ex.args {
                exporter_container["args"] = json!(args);
            }
            containers.push(exporter_container);
            pod_annotations = json!({
                "prometheus.io/scrape": "true",
                "prometheus.io/port": ex.port.to_string(),
                "prometheus.io/path": "/metrics"
            });
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
                        },
                        "annotations": pod_annotations
                    },
                    "spec": {
                        "containers": containers
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
                                // Uses the cluster default StorageClass (local-path on
                                // k3s) — dynamic provisioning. Immutable after create.
                                "storage": format!("{}Gi", storage_gb.max(1))
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

        Self::deploy_service(client, namespace, name, port, None).await?;
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

    #[tracing::instrument(skip_all, fields(namespace = %namespace, name = %name, replicas), err)]
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
            &PatchParams::default(),
            &Patch::Merge(&patch)
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
                tracing::warn!(pod = %pod_name, "Error getting logs for pod: {:?}", e);
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

    /// Run a one-off DB-client Job in the control-plane namespace with the shared
    /// `hermes-backups` PVC mounted at `/var/lib/hermes/backups`. Used for both
    /// backup (dump → file on the PVC) and restore (file on the PVC → DB).
    ///
    /// Unlike [`run_job_and_get_logs`], the Job mounts the backups PVC, so `command`
    /// can read/write dump files the control plane also sees (no streaming through
    /// pod logs, no size/binary truncation). The Job uses the database's own image,
    /// which ships the matching `pg_dump`/`psql`/`mysql`, and reaches the DB across
    /// namespaces via its FQDN — the per-workspace NetworkPolicy already allows
    /// ingress from `hermes-system`. The DB password is passed via a Secret (envFrom)
    /// so it never appears in the Job spec or logs. Returns the pod logs + exit code.
    ///
    /// NOTE: the backups PVC is ReadWriteOnce, so this Job pod must land on the same
    /// node as the control-plane pod that already mounts it. That holds on the
    /// single-node k3s target; a multi-node deploy would need node pinning here.
    pub async fn run_db_pvc_job(
        client: &Client,
        system_namespace: &str,
        name: &str,
        image: &str,
        envs: Vec<(String, String)>,
        command: &str,
        backups_pvc_claim: &str,
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

        let jobs: Api<k8s_openapi::api::batch::v1::Job> = Api::namespaced(client.clone(), system_namespace);

        // Clear any stale job with the same name to avoid a 409 on create.
        if jobs.get(name).await.is_ok() {
            let delete_params = DeleteParams {
                propagation_policy: Some(kube::api::PropagationPolicy::Background),
                ..Default::default()
            };
            let _ = jobs.delete(name, &delete_params).await;
            for _ in 0..10 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if jobs.get(name).await.is_err() {
                    break;
                }
            }
        }

        Self::create_secret(client, system_namespace, name, envs).await?;
        let _cleanup_guard = SecretCleanupGuard {
            client: client.clone(),
            namespace: system_namespace.to_string(),
            name: name.to_string(),
        };

        let job_manifest: k8s_openapi::api::batch::v1::Job = serde_json::from_value(json!({
            "apiVersion": "batch/v1",
            "kind": "Job",
            "metadata": { "name": name, "namespace": system_namespace },
            "spec": {
                "backoffLimit": 0,
                "ttlSecondsAfterFinished": 120,
                // Dumps can be slow on large DBs; give them room (30 min).
                "activeDeadlineSeconds": 1800,
                "template": {
                    "spec": {
                        "restartPolicy": "Never",
                        "containers": [{
                            "name": name,
                            "image": image,
                            "imagePullPolicy": "IfNotPresent",
                            "command": ["/bin/sh", "-c", command],
                            "resources": {
                                "requests": { "memory": "64Mi", "cpu": "100m" },
                                "limits": { "memory": "512Mi", "cpu": "1000m" }
                            },
                            "envFrom": [{ "secretRef": { "name": format!("{}-env", name) } }],
                            "volumeMounts": [{
                                "name": "backups",
                                "mountPath": "/var/lib/hermes/backups"
                            }]
                        }],
                        "volumes": [{
                            "name": "backups",
                            "persistentVolumeClaim": { "claimName": backups_pvc_claim }
                        }]
                    }
                }
            }
        })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Backup Job serialization failed: {}", e)))?;

        jobs.create(&PostParams::default(), &job_manifest).await
            .map_err(|e| AppError::Infrastructure(format!("Failed to create backup Job {}: {}", name, e)))?;

        // Wait for the Job to reach a terminal state.
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            match jobs.get(name).await {
                Ok(job_status) => {
                    if let Some(status) = job_status.status {
                        if status.succeeded.unwrap_or(0) > 0 || status.failed.unwrap_or(0) > 0 {
                            break;
                        }
                    }
                }
                Err(_) => return Err(AppError::Infrastructure(format!("Backup Job {} disappeared", name))),
            }
        }

        // Pull the pod's logs + exit code (for error surfacing).
        let pods_api: Api<Pod> = Api::namespaced(client.clone(), system_namespace);
        let lp = kube::api::ListParams::default().labels(&format!("job-name={}", name));
        let pod_list = pods_api.list(&lp).await
            .map_err(|e| AppError::Infrastructure(format!("Failed to list pods for backup Job: {}", e)))?;
        let (logs_str, exit_code) = match pod_list.items.first().and_then(|p| p.metadata.name.as_ref()) {
            Some(pod_name) => {
                let log_params = kube::api::LogParams { follow: false, ..Default::default() };
                let logs = pods_api.logs(pod_name, &log_params).await.unwrap_or_default();
                let code = pod_list.items.first()
                    .and_then(|p| p.status.as_ref())
                    .and_then(|s| s.container_statuses.as_ref())
                    .and_then(|st| st.first())
                    .and_then(|st| st.state.as_ref())
                    .and_then(|state| state.terminated.as_ref())
                    .map(|t| t.exit_code)
                    .unwrap_or(1);
                (logs, code)
            }
            None => (String::new(), 1),
        };

        let delete_params = DeleteParams {
            propagation_policy: Some(kube::api::PropagationPolicy::Background),
            ..Default::default()
        };
        let _ = jobs.delete(name, &delete_params).await;

        Ok((logs_str, exit_code))
    }
}
