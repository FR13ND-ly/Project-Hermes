use uuid::Uuid;
use sqlx::PgPool;
use kube::{Api, api::{PostParams, DeleteParams, PatchParams, Patch}};
use serde_json::json;
use base64::Engine as _;

use crate::models::app_model::AppStatus;

pub async fn run_ephemeral_build(
    pool: PgPool,
    instance_id: Uuid,
    git_repo: String,
    branch_name: String,
    build_cmd: Option<String>,
) {
    let start_instant = std::time::Instant::now();
    let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
        Ok(c) => c,
        Err(_) => {
            let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
            return;
        }
    };

    let meta = match sqlx::query!(
        "SELECT ai.container_name, ai.internal_port, ai.assigned_domain, a.id as app_id, a.project_id, a.workspace_id, ai.cpu_limit, ai.memory_limit_mb, u.github_token, a.start_command, a.git_subpath
         FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         JOIN workspaces w ON a.workspace_id = w.id
         JOIN users u ON w.created_by = u.id
         WHERE ai.id = $1", 
        instance_id
    )
    .fetch_optional(&pool)
    .await {
        Ok(Some(m)) => m,
        _ => {
            let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
            return;
        }
    };

    let limits = sqlx::query!("SELECT max_memory_mb, max_storage_gb FROM workspaces WHERE id = $1", meta.workspace_id)
        .fetch_one(&pool)
        .await;
    let (max_mem, max_storage) = match limits {
        Ok(r) => (r.max_memory_mb, r.max_storage_gb),
        Err(_) => (2048, 10),
    };
    let namespace = format!("hermes-ws-{}", meta.workspace_id);
    let _ = crate::utils::k8s::K8sManager::create_namespace(&k8s_client, &namespace, max_mem, max_storage).await;

    // Calculate currently used memory in Kubernetes to maximize builder headroom
    let mut total_used_mem = 0;
    let pods_api: Api<k8s_openapi::api::core::v1::Pod> = Api::namespaced(k8s_client.clone(), &namespace);
    if let Ok(pods_list) = pods_api.list(&kube::api::ListParams::default()).await {
        for pod in pods_list.items {
            if let Some(ref name) = pod.metadata.name {
                if name.starts_with("hermes-builder-") {
                    continue;
                }
            }
            if let Some(spec) = pod.spec {
                for container in spec.containers {
                    if let Some(resources) = container.resources {
                        if let Some(limits) = resources.limits {
                            if let Some(mem_qty) = limits.get("memory") {
                                total_used_mem += parse_memory_quantity(&mem_qty.0);
                            }
                        }
                    }
                }
            }
        }
    }

    let builder_mem_limit = std::cmp::max(512, max_mem - total_used_mem);

    let mut commit_sha = None;
    let mut commit_msg = None;

    if git_repo.contains("github.com") {
        if let Some(ref token) = meta.github_token {
            let clean_repo = git_repo
                .trim()
                .replace("https://github.com/", "")
                .replace("git@github.com:", "")
                .replace(".git", "");
            let parts: Vec<&str> = clean_repo.split('/').collect();
            if parts.len() >= 2 {
                let owner = parts[0];
                let repo_name = parts[1];
                
                let client = reqwest::Client::new();
                let url = format!("https://api.github.com/repos/{}/{}/commits/{}", owner, repo_name, branch_name);
                
                #[derive(Debug, serde::Deserialize)]
                struct GitHubCommitInfo {
                    sha: String,
                    commit: CommitData,
                }
                #[derive(Debug, serde::Deserialize)]
                struct CommitData {
                    message: String,
                }
                
                if let Ok(res) = client.get(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .header("User-Agent", "hermes-orchestrator")
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                {
                    if res.status().is_success() {
                        if let Ok(commit_info) = res.json::<GitHubCommitInfo>().await {
                            commit_sha = Some(commit_info.sha);
                            commit_msg = Some(commit_info.commit.message);
                        }
                    }
                }
            }
        }
    }

    let build_id = Uuid::new_v4();
    let _ = sqlx::query!(
        "INSERT INTO app_builds (id, app_id, app_instance_id, status, logs, commit_message, commit_sha) VALUES ($1, $2, $3, 'building', '', $4, $5)",
        build_id, meta.app_id, instance_id, commit_msg, commit_sha
    )
    .execute(&pool)
    .await;

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
            workspace_id: meta.workspace_id,
            build_id,
            app_id: meta.app_id,
            status: "building".to_string(),
        }
    );

    let registry_url = std::env::var("HERMES_REGISTRY_URL").unwrap_or_else(|_| "localhost:5000".to_string());
    let full_image_tag = format!("{}/hermes-app-image:{}", registry_url, instance_id);
    let builder_pod_name = format!("hermes-builder-{}", instance_id);

    // For Kaniko running inside the cluster, localhost/127.0.0.1 registry must be accessed via the internal registry service
    let mut kaniko_destination = full_image_tag.clone();
    if registry_url.contains("localhost") || registry_url.contains("127.0.0.1") {
        kaniko_destination = format!("registry.kube-system.svc.cluster.local:80/hermes-app-image:{}", instance_id);
    }

    // Set up private registry credentials if configured
    let registry_user = std::env::var("HERMES_REGISTRY_USER").ok();
    let registry_password = std::env::var("HERMES_REGISTRY_PASSWORD").ok();
    let mut has_registry_creds = false;

    if let (Some(user), Some(pass)) = (registry_user, registry_password) {
        if !user.is_empty() && !pass.is_empty() {
            let auth_bytes = format!("{}:{}", user, pass);
            let encoded_auth = base64::engine::general_purpose::STANDARD.encode(auth_bytes.as_bytes());
            let docker_config = json!({
                "auths": {
                    registry_url: {
                        "auth": encoded_auth
                    },
                    "registry.kube-system.svc.cluster.local:80": {
                        "auth": encoded_auth
                    }
                }
            });
            let docker_config_str = docker_config.to_string();

            let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
            let secret_manifest: k8s_openapi::api::core::v1::Secret = serde_json::from_value(json!({
                "apiVersion": "v1",
                "kind": "Secret",
                "metadata": {
                    "name": "hermes-registry-credentials",
                    "namespace": namespace
                },
                "type": "kubernetes.io/dockerconfigjson",
                "stringData": {
                    ".dockerconfigjson": docker_config_str
                }
            })).unwrap();

            let _ = secrets.patch(
                "hermes-registry-credentials",
                &PatchParams::apply("hermes-orchestrator").force(),
                &Patch::Apply(&secret_manifest)
            ).await;
            has_registry_creds = true;
        }
    }

    // Set up project SSH keys if configured
    let ssh_keys = sqlx::query!(
        "SELECT host, encrypted_private_key, nonce FROM project_ssh_keys WHERE project_id = $1",
        meta.project_id
    )
    .fetch_all(&pool)
    .await
    .unwrap_or_default();

    let mut keys_to_mount = Vec::new();
    for key in ssh_keys {
        if let Ok(decrypted) = crate::utils::crypto::decrypt_env_value(&key.encrypted_private_key, &key.nonce) {
            keys_to_mount.push((key.host, decrypted));
        }
    }
    let has_ssh_keys = !keys_to_mount.is_empty();
    let ssh_secret_name = format!("hermes-ssh-keys-{}", instance_id);

    if has_ssh_keys {
        let mut string_data = serde_json::Map::new();
        for (host, key) in &keys_to_mount {
            let key_name = format!("key-{}", host.replace(":", "_"));
            string_data.insert(key_name, json!(key));
        }

        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
        let secret_manifest: k8s_openapi::api::core::v1::Secret = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": ssh_secret_name,
                "namespace": namespace
            },
            "type": "Opaque",
            "stringData": string_data
        })).unwrap();

        let _ = secrets.patch(
            &ssh_secret_name,
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&secret_manifest)
        ).await;
    }

    let mut cloner_repo = git_repo.clone();
    if cloner_repo.starts_with("https://github.com/") {
        if let Some(ref token) = meta.github_token {
            if !token.trim().is_empty() {
                cloner_repo = cloner_repo.replace("https://github.com/", &format!("https://x-access-token:{}@github.com/", token.trim()));
            }
        }
    }

    let build_instruction = match build_cmd {
        Some(ref cmd) if !cmd.trim().is_empty() => format!("RUN {}", cmd.trim()),
        _ => "".to_string(),
    };

    let rust_build_instruction = match build_cmd {
        Some(ref cmd) if !cmd.trim().is_empty() => format!("RUN {}", cmd.trim()),
        _ => "RUN cargo build --release".to_string(),
    };

    let spa_build_instruction = match build_cmd {
        Some(ref cmd) if !cmd.trim().is_empty() => format!("RUN {}", cmd.trim()),
        _ => "RUN npm run build".to_string(),
    };

    let start_cmd = meta.start_command.as_deref().unwrap_or("").trim();

    let node_start = if !start_cmd.is_empty() {
        format!("CMD {}", start_cmd)
    } else {
        "CMD [\"npm\", \"start\"]".to_string()
    };

    let python_start = if !start_cmd.is_empty() {
        format!("CMD {}", start_cmd)
    } else {
        "CMD [\"python\", \"main.py\"]".to_string()
    };

    let rust_start = if !start_cmd.is_empty() {
        format!("CMD {}", start_cmd)
    } else {
        "CMD [\"cargo\", \"run\", \"--release\"]".to_string()
    };

    let fallback_start = if !start_cmd.is_empty() {
        format!("CMD {}", start_cmd)
    } else {
        "CMD [\"npm\", \"start\"]".to_string()
    };

    let internal_port = meta.internal_port;
    let sub_path = meta.git_subpath.as_deref().unwrap_or("").trim().trim_matches('/');

    let change_dir_and_detect = if !sub_path.is_empty() {
        format!(
            r#"cd /workspace
if [ -d "{sub_path}" ]; then
  cd "{sub_path}"
  echo "Folosim subdirectorul: {sub_path}"
else
  echo "Eroare: Subdirectorul {sub_path} nu a fost găsit în repository!" >&2
  exit 1
fi"#,
            sub_path = sub_path
        )
    } else {
        "cd /workspace".to_string()
    };

    let mut ssh_setup_script = String::new();
    if has_ssh_keys {
        ssh_setup_script.push_str("mkdir -p ~/.ssh\n");
        ssh_setup_script.push_str("chmod 700 ~/.ssh\n");
        ssh_setup_script.push_str("cat << 'EOF' > ~/.ssh/config\n");
        ssh_setup_script.push_str("StrictHostKeyChecking no\n");
        ssh_setup_script.push_str("UserKnownHostsFile /dev/null\n");
        ssh_setup_script.push_str("EOF\n\n");
        
        for (host, _) in &keys_to_mount {
            let key_name = format!("key-{}", host.replace(":", "_"));
            ssh_setup_script.push_str(&format!("cp /var/git-ssh-keys/{} ~/.ssh/{}\n", key_name, key_name));
            ssh_setup_script.push_str(&format!("chmod 600 ~/.ssh/{}\n", key_name));
            ssh_setup_script.push_str(&format!("cat << 'EOF' >> ~/.ssh/config\n"));
            ssh_setup_script.push_str(&format!("Host {}\n", host));
            ssh_setup_script.push_str(&format!("  HostName {}\n", host));
            ssh_setup_script.push_str(&format!("  IdentityFile ~/.ssh/{}\n", key_name));
            ssh_setup_script.push_str(&format!("  IdentitiesOnly yes\n"));
            ssh_setup_script.push_str("EOF\n\n");
        }
        ssh_setup_script.push_str("export GIT_SSH_COMMAND=\"ssh -F ~/.ssh/config\"\n");
    }

    let cloner_script = format!(
        r#"{ssh_setup_script}git clone --depth 1 --branch {branch_name} {cloner_repo} /workspace
{change_dir_and_detect}
if [ ! -f Dockerfile ]; then
  echo "No Dockerfile found, generating fallback..."
  if [ -f package.json ]; then
    echo "Detected Node.js project"
    if (grep -q "@angular/core" package.json || grep -q '"react"' package.json || grep -q '"vue"' package.json || grep -q '"svelte"' package.json) && ! grep -q '"next"' package.json && ! grep -q '"nuxt"' package.json; then
      echo "Detected Client-side SPA Frontend project. Serving with Nginx..."
      cat << 'EOF' > nginx.conf
server {{
    listen {internal_port};
    location / {{
        root /usr/share/nginx/html;
        index index.html index.htm;
        try_files $uri $uri/ /index.html;
    }}
}}
EOF
      cat << 'EOF' > Dockerfile
FROM node:20-alpine AS builder
WORKDIR /app
COPY . .
RUN npm install
{spa_build_instruction}
RUN OUT_DIR=$(find dist build -name index.html -exec dirname {{}} \; | head -n 1) && \
    mkdir -p /app/public_html && \
    cp -r $OUT_DIR/* /app/public_html/

FROM nginx:alpine
COPY --from=builder /app/public_html /usr/share/nginx/html
COPY nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE {internal_port}
CMD ["nginx", "-g", "daemon off;"]
EOF
    else
      cat << 'EOF' > Dockerfile
FROM node:20-alpine
ENV PORT {internal_port}
WORKDIR /app
COPY . .
RUN npm install
{build_instruction}
EXPOSE {internal_port}
{node_start}
EOF
    fi
  elif [ -f requirements.txt ] || [ -f main.py ] || [ -f setup.py ]; then
    echo "Detected Python project"
    cat << 'EOF' > Dockerfile
FROM python:3.11-slim
ENV PORT {internal_port}
WORKDIR /app
COPY . .
RUN if [ -f requirements.txt ]; then pip install --no-cache-dir -r requirements.txt; fi
{build_instruction}
EXPOSE {internal_port}
{python_start}
EOF
  elif [ -f Cargo.toml ]; then
    echo "Detected Rust project"
    cat << 'EOF' > Dockerfile
FROM rust:1.75
ENV PORT {internal_port}
WORKDIR /app
COPY . .
{rust_build_instruction}
EXPOSE {internal_port}
{rust_start}
EOF
  else
    echo "Fallback to default Node.js template"
    cat << 'EOF' > Dockerfile
FROM node:20-alpine
ENV PORT {internal_port}
WORKDIR /app
COPY . .
RUN if [ -f package.json ]; then npm install; fi
{build_instruction}
EXPOSE {internal_port}
{fallback_start}
EOF
  fi
fi"#,
        ssh_setup_script = ssh_setup_script,
        branch_name = branch_name,
        cloner_repo = cloner_repo,
        change_dir_and_detect = change_dir_and_detect,
        build_instruction = build_instruction,
        rust_build_instruction = rust_build_instruction,
        spa_build_instruction = spa_build_instruction,
        internal_port = internal_port,
        node_start = node_start,
        python_start = python_start,
        rust_start = rust_start,
        fallback_start = fallback_start,
    );

    let context_dir = if !sub_path.is_empty() {
        format!("dir:///workspace/{}", sub_path)
    } else {
        "dir:///workspace".to_string()
    };

    let mut builder_pod_manifest = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": builder_pod_name,
            "namespace": namespace,
            "labels": {
                "app": "hermes-builder",
                "instance-id": instance_id.to_string()
            }
        },
        "spec": {
            "restartPolicy": "Never",
            "initContainers": [{
                "name": "cloner",
                "image": "alpine/git:latest",
                "command": ["/bin/sh", "-c", cloner_script],
                "volumeMounts": [{
                    "name": "workspace",
                    "mountPath": "/workspace"
                }]
            }],
            "containers": [{
                "name": "kaniko",
                "image": "gcr.io/kaniko-project/executor:v1.14.0",
                "args": [
                    format!("--context={}", context_dir),
                    "--dockerfile=Dockerfile",
                    format!("--destination={}", kaniko_destination),
                    "--skip-tls-verify",
                    "--insecure"
                ],
                "volumeMounts": [{
                    "name": "workspace",
                    "mountPath": "/workspace"
                }],
                "resources": {
                    "requests": {
                        "cpu": "200m",
                        "memory": "512Mi"
                    },
                    "limits": {
                        "cpu": "2000m",
                        "memory": format!("{}Mi", builder_mem_limit)
                    }
                }
            }],
            "volumes": [{
                "name": "workspace",
                "emptyDir": {}
            }]
        }
    });

    if has_registry_creds {
        if let Some(spec) = builder_pod_manifest.get_mut("spec") {
            if let Some(containers) = spec.get_mut("containers") {
                if let Some(kaniko_container) = containers.get_mut(0) {
                    if let Some(mounts) = kaniko_container.get_mut("volumeMounts") {
                        if let Some(mounts_arr) = mounts.as_array_mut() {
                            mounts_arr.push(json!({
                                "name": "registry-creds",
                                "mountPath": "/kaniko/.docker"
                            }));
                        }
                    }
                }
            }
            if let Some(volumes) = spec.get_mut("volumes") {
                if let Some(volumes_arr) = volumes.as_array_mut() {
                    volumes_arr.push(json!({
                        "name": "registry-creds",
                        "secret": {
                            "secretName": "hermes-registry-credentials"
                        }
                    }));
                }
            }
        }
    }

    if has_ssh_keys {
        if let Some(spec) = builder_pod_manifest.get_mut("spec") {
            if let Some(init_containers) = spec.get_mut("initContainers") {
                if let Some(cloner) = init_containers.get_mut(0) {
                    if let Some(mounts) = cloner.get_mut("volumeMounts") {
                        if let Some(mounts_arr) = mounts.as_array_mut() {
                            mounts_arr.push(json!({
                                "name": "git-ssh-keys",
                                "mountPath": "/var/git-ssh-keys",
                                "readOnly": true
                            }));
                        }
                    }
                }
            }
            if let Some(volumes) = spec.get_mut("volumes") {
                if let Some(volumes_arr) = volumes.as_array_mut() {
                    volumes_arr.push(json!({
                        "name": "git-ssh-keys",
                        "secret": {
                            "secretName": ssh_secret_name
                        }
                    }));
                }
            }
        }
    }

    let pod_manifest: k8s_openapi::api::core::v1::Pod = match serde_json::from_value(builder_pod_manifest) {
        Ok(p) => p,
        Err(e) => {
            let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
            let error_msg = format!("Eroare la generarea manifestului pod-ului de build: {}", e);
            let duration_sec = start_instant.elapsed().as_secs() as i32;
            let _ = sqlx::query!(
                "UPDATE app_builds SET status = 'failed', logs = $1, duration_sec = $2 WHERE id = $3",
                error_msg, duration_sec, build_id
            )
            .execute(&pool)
            .await;

            crate::utils::event_broadcaster::broadcast_event(
                crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
                    workspace_id: meta.workspace_id,
                    build_id,
                    app_id: meta.app_id,
                    status: "failed".to_string(),
                }
            );

            return;
        }
    };

    let pods: Api<k8s_openapi::api::core::v1::Pod> = Api::namespaced(k8s_client.clone(), &namespace);

    if let Err(e) = pods.create(&PostParams::default(), &pod_manifest).await {
        let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
        let error_msg = format!(
            "Eroare la crearea pod-ului de build în Kubernetes (verifică cota de resurse a workspace-ului):\n{}",
            e
        );
        let duration_sec = start_instant.elapsed().as_secs() as i32;
        let _ = sqlx::query!(
            "UPDATE app_builds SET status = 'failed', logs = $1, duration_sec = $2 WHERE id = $3",
            error_msg, duration_sec, build_id
        )
        .execute(&pool)
        .await;

        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
                workspace_id: meta.workspace_id,
                build_id,
                app_id: meta.app_id,
                status: "failed".to_string(),
            }
        );

        if has_registry_creds {
            let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
            let _ = secrets.delete("hermes-registry-credentials", &DeleteParams::default()).await;
        }
        if has_ssh_keys {
            let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
            let _ = secrets.delete(&ssh_secret_name, &DeleteParams::default()).await;
        }
        return;
    }

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
    let mut success = false;
    
    // Max 10 minutes timeout (300 ticks of 2 seconds)
    for _ in 0..300 {
        interval.tick().await;
        if let Ok(pod) = pods.get(&builder_pod_name).await {
            if let Some(status) = pod.status {
                if let Some(phase) = status.phase {
                    if phase == "Succeeded" {
                        success = true;
                        break;
                    }
                    if phase == "Failed" {
                        break;
                    }
                }
            }
        } else {
            break;
        }
    }

    // Capture builder pod logs from both cloner and build steps
    let mut cloner_logs = String::new();
    let cloner_params = kube::api::LogParams {
        container: Some("cloner".to_string()),
        ..Default::default()
    };
    match pods.logs(&builder_pod_name, &cloner_params).await {
        Ok(logs) => cloner_logs.push_str(&logs),
        Err(e) => cloner_logs.push_str(&format!("Nu s-au putut prelua logurile pentru cloner: {}\n", e)),
    }

    let mut kaniko_logs = String::new();
    let kaniko_params = kube::api::LogParams {
        container: Some("kaniko".to_string()),
        ..Default::default()
    };
    match pods.logs(&builder_pod_name, &kaniko_params).await {
        Ok(logs) => kaniko_logs.push_str(&logs),
        Err(e) => kaniko_logs.push_str(&format!("Nu s-au putut prelua logurile pentru Kaniko/Build: {}\n", e)),
    }

    let mut cloner_duration_str = "N/A".to_string();
    let mut kaniko_duration_str = "N/A".to_string();
    
    if let Ok(pod) = pods.get(&builder_pod_name).await {
        if let Some(status) = pod.status {
            if let Some(init_statuses) = status.init_container_statuses {
                if let Some(cloner_status) = init_statuses.iter().find(|c| c.name == "cloner") {
                    if let Some(ref state) = cloner_status.state {
                        if let Some(ref terminated) = state.terminated {
                            if let (Some(started), Some(finished)) = (&terminated.started_at, &terminated.finished_at) {
                                let duration = finished.0.signed_duration_since(started.0);
                                cloner_duration_str = format!("{}s", duration.num_seconds());
                            }
                        }
                    }
                }
            }
            if let Some(cont_statuses) = status.container_statuses {
                if let Some(kaniko_status) = cont_statuses.iter().find(|c| c.name == "kaniko") {
                    if let Some(ref state) = kaniko_status.state {
                        if let Some(ref terminated) = state.terminated {
                            if let (Some(started), Some(finished)) = (&terminated.started_at, &terminated.finished_at) {
                                let duration = finished.0.signed_duration_since(started.0);
                                kaniko_duration_str = format!("{}s", duration.num_seconds());
                            }
                        }
                    }
                }
            }
        }
    }

    // Clean up builder pod
    let _ = pods.delete(&builder_pod_name, &DeleteParams::default()).await;

    // Clean up registry credentials secret if created
    if has_registry_creds {
        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = secrets.delete("hermes-registry-credentials", &DeleteParams::default()).await;
    }

    if has_ssh_keys {
        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = secrets.delete(&ssh_secret_name, &DeleteParams::default()).await;
    }

    let mut build_logs = String::new();
    build_logs.push_str("=========================================\n");
    build_logs.push_str(&format!(" ETAPA 1: DESCĂRCARE COD (GIT CLONE) [Durată: {}]\n", cloner_duration_str));
    build_logs.push_str("=========================================\n");
    build_logs.push_str(&cloner_logs);

    build_logs.push_str("\n\n=========================================\n");
    build_logs.push_str(&format!(" ETAPA 2: CONSTRUIRE IMAGINE (KANIKO) [Durată: {}]\n", kaniko_duration_str));
    build_logs.push_str("=========================================\n");
    build_logs.push_str(&kaniko_logs);

    let total_build_duration_str = format!("{}s", start_instant.elapsed().as_secs());

    if success {
        build_logs.push_str("\n\n=========================================\n");
        build_logs.push_str(&format!(" ETAPA 3: CONSTRUIRE REUȘITĂ (SUCCESS) [Timp Total Build: {}]\n", total_build_duration_str));
        build_logs.push_str("=========================================\n");
        build_logs.push_str("Imaginea Docker a fost creată cu succes și trimisă în registry.\n");
        build_logs.push_str("Se pornește faza de lansare în clusterul Kubernetes...\n");
    } else {
        build_logs.push_str("\n\n=========================================\n");
        build_logs.push_str(&format!(" ETAPA 3: CONSTRUIRE EȘUATĂ (FAILED) [Timp Total Build: {}]\n", total_build_duration_str));
        build_logs.push_str("=========================================\n");
        build_logs.push_str("Construirea imaginii a eșuat. Consultă logurile de mai sus pentru detalii.\n");
    }

    let status_str = if success { "succeeded" } else { "failed" };
    let duration_sec = start_instant.elapsed().as_secs() as i32;
    let _ = sqlx::query!(
        "UPDATE app_builds SET status = $1, logs = $2, duration_sec = $3 WHERE id = $4",
        status_str, build_logs, duration_sec, build_id
    )
    .execute(&pool)
    .await;

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
            workspace_id: meta.workspace_id,
            build_id,
            app_id: meta.app_id,
            status: status_str.to_string(),
        }
    );

    if !success {
        let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
        return;
    }

    deploy_compiled_app(pool, instance_id, full_image_tag).await;
}

pub async fn deploy_compiled_app(pool: PgPool, instance_id: Uuid, image_tag: String) {
    let deploy_start_instant = std::time::Instant::now();
    let mut deployment_image = image_tag.clone();
    if let Ok(reg_url) = std::env::var("HERMES_REGISTRY_URL") {
        if deployment_image.starts_with(&reg_url) {
            if reg_url.contains("192.168.") || reg_url.contains("127.0.0.1") || reg_url.contains("localhost") {
                deployment_image = deployment_image.replace(&reg_url, "localhost:5000");
            }
        }
    }

    let instance_meta = sqlx::query!(
        "SELECT ai.container_name, ai.internal_port, ai.external_port, ai.assigned_domain, a.id as app_id, a.project_id, a.workspace_id, ai.cpu_limit, ai.memory_limit_mb, a.tcp_udp_ports, ai.meta_data 
         FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         WHERE ai.id = $1", 
        instance_id
    )
    .fetch_optional(&pool)
    .await;

    if let Ok(Some(meta)) = instance_meta {
        let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
            Ok(c) => c,
            Err(_) => {
                let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
                return;
            }
        };

        let limits = sqlx::query!("SELECT max_memory_mb, max_storage_gb FROM workspaces WHERE id = $1", meta.workspace_id)
            .fetch_one(&pool)
            .await;
        let (max_mem, max_storage) = match limits {
            Ok(r) => (r.max_memory_mb, r.max_storage_gb),
            Err(_) => (2048, 10),
        };
        let namespace = format!("hermes-ws-{}", meta.workspace_id);
        let _ = crate::utils::k8s::K8sManager::create_namespace(&k8s_client, &namespace, max_mem, max_storage).await;

        let mut envs = Vec::new();
        let env_records = sqlx::query!(
            "SELECT key, encrypted_value, nonce, is_secret FROM environment_variables 
             WHERE workspace_id = $1 
               AND (project_id = $2 OR project_id IS NULL)
               AND (app_instance_id = $3 OR app_instance_id IS NULL)",
            meta.workspace_id, meta.project_id, instance_id
        )
        .fetch_all(&pool)
        .await;

        if let Ok(records) = env_records {
            for rec in records {
                if let Ok(decrypted_value) = crate::utils::crypto::decrypt_env_value(&rec.encrypted_value, &rec.nonce) {
                    envs.push((rec.key, decrypted_value));
                }
            }
        }

        let volume_records = sqlx::query!(
            "SELECT container_path, host_path FROM app_volumes WHERE app_id = $1",
            meta.app_id
        )
        .fetch_all(&pool)
        .await;

        let mut binds = Vec::new();
        if let Ok(volumes) = volume_records {
            for vol in volumes {
                binds.push((vol.host_path, vol.container_path));
            }
        }
        let app_name = &meta.container_name;
        let cpu_limit = meta.cpu_limit.unwrap_or(0);
        let memory_limit_mb = meta.memory_limit_mb.unwrap_or(0);
        let meta_data = &meta.meta_data;
        let knative_enabled = meta_data.get("knative_enabled").and_then(|v| v.as_bool()).unwrap_or(false);

        if knative_enabled {
            let min_scale = meta_data.get("minScale").and_then(|v| v.as_i64()).or_else(|| meta_data.get("min_scale").and_then(|v| v.as_i64())).unwrap_or(0) as i32;
            let max_scale = meta_data.get("maxScale").and_then(|v| v.as_i64()).or_else(|| meta_data.get("max_scale").and_then(|v| v.as_i64())).unwrap_or(5) as i32;
            let target_concurrency = meta_data.get("targetConcurrency").and_then(|v| v.as_i64()).or_else(|| meta_data.get("target_concurrency").and_then(|v| v.as_i64())).unwrap_or(10) as i32;

            // Cleanup standard K8s Deployment to avoid conflict
            let _ = crate::utils::k8s::K8sManager::delete_app(&k8s_client, &namespace, app_name).await;

            if crate::utils::k8s::K8sManager::deploy_knative_service(
                &k8s_client,
                &namespace,
                app_name,
                &deployment_image,
                envs,
                min_scale,
                max_scale,
                target_concurrency,
                Some(memory_limit_mb as i32),
            ).await.is_ok() {
                
                if let Some(ref domain) = meta.assigned_domain {
                    let _ = crate::utils::k8s::K8sManager::deploy_ingress(
                        &k8s_client,
                        &namespace,
                        app_name,
                        domain,
                        app_name,
                        80 // Knative service always listens on port 80 internally
                    ).await;
                }

                let _ = update_status(&pool, instance_id, AppStatus::Running).await;

                let deploy_duration_str = format!("{}s", deploy_start_instant.elapsed().as_secs());

                if let Ok(Some(build_rec)) = sqlx::query!(
                    "SELECT id, logs FROM app_builds WHERE app_instance_id = $1 ORDER BY created_at DESC LIMIT 1",
                    instance_id
                )
                .fetch_optional(&pool)
                .await {
                    let mut updated_logs = build_rec.logs;
                    updated_logs.push_str("\n=========================================\n");
                    updated_logs.push_str(&format!(" ETAPA 4: DEPLOY REUȘIT (SERVERLESS) [Durată: {}]\n", deploy_duration_str));
                    updated_logs.push_str("=========================================\n");
                    updated_logs.push_str(&format!("- Namespace: {} -> OK\n", namespace));
                    updated_logs.push_str(&format!("- Knative Service: {} (Min Scale: {}, Max Scale: {}, Concurrency: {}) -> OK\n", app_name, min_scale, max_scale, target_concurrency));
                    if let Some(ref domain) = meta.assigned_domain {
                        updated_logs.push_str(&format!("- Ingress Domeniu: http://{} -> OK\n", domain));
                    }
                    updated_logs.push_str("\n=========================================\n");
                    updated_logs.push_str(" SERVICIUL SERVERLESS A FOST LANSAT ȘI ESTE ACTIV!\n");
                    updated_logs.push_str("=========================================\n");
                    
                    let _ = sqlx::query!(
                        "UPDATE app_builds SET logs = $1 WHERE id = $2",
                        updated_logs, build_rec.id
                    )
                    .execute(&pool)
                    .await;
                }
                return; // SUCCESS!
            }
        } else {
            // Cleanup Knative service if transitioning back to standard
            let _ = crate::utils::k8s::K8sManager::delete_knative_service(&k8s_client, &namespace, app_name).await;

            if crate::utils::k8s::K8sManager::deploy_app(
                &k8s_client,
                &namespace,
                app_name,
                &deployment_image,
                meta.internal_port,
                envs,
                binds,
                cpu_limit,
                memory_limit_mb
            ).await.is_ok() {
                
                if crate::utils::k8s::K8sManager::deploy_service(
                    &k8s_client,
                    &namespace,
                    app_name,
                    meta.internal_port
                ).await.is_ok() {
                    if let Some(ports_arr) = meta.tcp_udp_ports.as_array() {
                        for (i, p) in ports_arr.iter().enumerate() {
                            if let (Some(int_p), Some(ext_p)) = (p.get("internal").and_then(|ip| ip.as_i64()), p.get("external").and_then(|ep| ep.as_i64())) {
                                let proto = p.get("protocol").and_then(|pr| pr.as_str()).unwrap_or("TCP");
                                let lb_name = format!("{}-port-{}", app_name, i);
                                let _ = crate::utils::k8s::K8sManager::deploy_loadbalancer_service(
                                    &k8s_client,
                                    &namespace,
                                    &lb_name,
                                    app_name,
                                    int_p as i32,
                                    ext_p as i32,
                                    proto,
                                ).await;
                            }
                        }
                    }

                    if let Some(ext_port) = meta.external_port {
                        let lb_name = format!("{}-external", app_name);
                        let _ = crate::utils::k8s::K8sManager::deploy_loadbalancer_service(
                            &k8s_client,
                            &namespace,
                            &lb_name,
                            app_name,
                            meta.internal_port,
                            ext_port,
                            "TCP",
                        ).await;
                    }

                    if let Some(ref domain) = meta.assigned_domain {
                        let _ = crate::utils::k8s::K8sManager::deploy_ingress(
                            &k8s_client,
                            &namespace,
                            app_name,
                            domain,
                            app_name,
                            meta.internal_port
                        ).await;
                    }

                    let _ = update_status(&pool, instance_id, AppStatus::Running).await;

                    let deploy_duration_str = format!("{}s", deploy_start_instant.elapsed().as_secs());

                    if let Ok(Some(build_rec)) = sqlx::query!(
                        "SELECT id, logs FROM app_builds WHERE app_instance_id = $1 ORDER BY created_at DESC LIMIT 1",
                        instance_id
                    )
                    .fetch_optional(&pool)
                    .await {
                        let mut updated_logs = build_rec.logs;
                        updated_logs.push_str("\n=========================================\n");
                        updated_logs.push_str(&format!(" ETAPA 4: DEPLOY REUȘIT (DEPLOYED) [Durată: {}]\n", deploy_duration_str));
                        updated_logs.push_str("=========================================\n");
                        updated_logs.push_str(&format!("- Namespace: {} -> OK\n", namespace));
                        updated_logs.push_str(&format!("- Deployment: {} (Port Intern: {}) -> OK\n", app_name, meta.internal_port));
                        if let Some(ext_port) = meta.external_port {
                            updated_logs.push_str(&format!("- Serviciu LoadBalancer: port extern {} -> OK\n", ext_port));
                            updated_logs.push_str(&format!("  -> Accesibil la: http://localhost:{}\n", ext_port));
                        }
                        if let Some(ref domain) = meta.assigned_domain {
                            updated_logs.push_str(&format!("- Rute Ingress: http://{} -> OK\n", domain));
                        }
                        updated_logs.push_str("\n=========================================\n");
                        updated_logs.push_str(" APLICAȚIA A FOST LANSATĂ ȘI ESTE ACTIVĂ!\n");
                        updated_logs.push_str("=========================================\n");
                        
                        let _ = sqlx::query!(
                            "UPDATE app_builds SET logs = $1 WHERE id = $2",
                            updated_logs, build_rec.id
                        )
                        .execute(&pool)
                        .await;
                    }
                    return; // SUCCESS!
                }
            }
        }
    }

    // Log failed deployment to latest build record
    let deploy_duration_str = format!("{}s", deploy_start_instant.elapsed().as_secs());
    if let Ok(Some(build_rec)) = sqlx::query!(
        "SELECT id, logs FROM app_builds WHERE app_instance_id = $1 ORDER BY created_at DESC LIMIT 1",
        instance_id
    )
    .fetch_optional(&pool)
    .await {
        let mut updated_logs = build_rec.logs;
        updated_logs.push_str("\n\n=========================================\n");
        updated_logs.push_str(&format!(" ETAPA 4: DEPLOY EȘUAT (DEPLOYMENT FAILED) [Durată: {}]\n", deploy_duration_str));
        updated_logs.push_str("=========================================\n");
        updated_logs.push_str("Eroare la provizionarea resurselor Kubernetes în cluster.\n");

        let _ = sqlx::query!(
            "UPDATE app_builds SET logs = $1, status = 'failed' WHERE id = $2",
            updated_logs, build_rec.id
        )
        .execute(&pool)
        .await;
    }

    let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
}

async fn update_status(pool: &sqlx::PgPool, id: Uuid, status: AppStatus) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE app_instances SET status = $1, updated_at = now() WHERE id = $2", status.clone() as AppStatus, id)
        .execute(pool)
        .await?;

    if let Ok(Some(meta)) = sqlx::query!(
        "SELECT a.workspace_id, ai.container_name FROM app_instances ai JOIN apps a ON ai.app_id = a.id WHERE ai.id = $1",
        id
    )
    .fetch_optional(pool)
    .await {
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::InstanceStatusChanged {
                workspace_id: meta.workspace_id,
                instance_id: id,
                container_name: meta.container_name,
                status: format!("{:?}", status).to_lowercase(),
            }
        );
    }

    Ok(())
}

fn parse_memory_quantity(qty_str: &str) -> i32 {
    let qty_str = qty_str.trim();
    if qty_str.ends_with("Gi") {
        qty_str.replace("Gi", "").parse::<i32>().unwrap_or(0) * 1024
    } else if qty_str.ends_with("Mi") {
        qty_str.replace("Mi", "").parse::<i32>().unwrap_or(0)
    } else if qty_str.ends_with("Ki") {
        qty_str.replace("Ki", "").parse::<i32>().unwrap_or(0) / 1024
    } else {
        qty_str.parse::<i64>().unwrap_or(0) as i32 / 1024 / 1024
    }
}