use uuid::Uuid;
use sqlx::PgPool;
use kube::{Api, api::{PostParams, DeleteParams, PatchParams, Patch}};
use serde_json::json;
use base64::Engine as _;
use std::sync::OnceLock;
use tokio::sync::Semaphore;

use crate::models::app_model::AppStatus;

/// Cluster-wide cap on simultaneous image builds. Builds beyond the cap stay in
/// the 'queued' phase until a slot frees up, preventing the cluster from being
/// overwhelmed by many concurrent Kaniko pods. Configurable via env.
static BUILD_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

fn build_semaphore() -> &'static Semaphore {
    BUILD_SEMAPHORE.get_or_init(|| {
        let permits = std::env::var("HERMES_MAX_CONCURRENT_BUILDS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(3);
        Semaphore::new(permits)
    })
}

/// Number of build-concurrency permits currently free (0 = builds saturated).
/// Sampled by the metrics gauge sampler.
pub fn available_build_permits() -> usize {
    build_semaphore().available_permits()
}

/// Read a build's current phase from the database (used to detect cancellation
/// and supersession, which are signalled by writing to the `phase` column).
async fn build_phase_db(pool: &sqlx::PgPool, build_id: Uuid) -> Option<String> {
    sqlx::query_scalar!("SELECT phase FROM app_builds WHERE id = $1", build_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

#[tracing::instrument(skip_all, fields(instance_id = %instance_id, branch = %branch_name))]
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
        "SELECT ai.container_name, ai.internal_port, ai.assigned_domain, a.id as app_id, a.project_id, a.workspace_id, ai.cpu_limit, ai.memory_limit_mb, u.github_token, a.start_command, a.git_subpath, a.git_credential_id
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

    tracing::info!(repo = %git_repo, "Build started");

    let limits = sqlx::query!("SELECT max_memory_mb, max_storage_gb, max_cpu_millicores FROM workspaces WHERE id = $1", meta.workspace_id)
        .fetch_one(&pool)
        .await;
    let (max_mem, max_storage, max_cpu) = match limits {
        Ok(r) => (r.max_memory_mb, r.max_storage_gb, r.max_cpu_millicores),
        Err(_) => (2048, 10, 0),
    };
    let namespace = format!("hermes-ws-{}", meta.workspace_id);
    let _ = crate::utils::k8s::K8sManager::create_namespace(&k8s_client, &namespace, max_mem, max_storage, max_cpu).await;

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
                                total_used_mem += crate::utils::quantity::parse_memory_mib(&mem_qty.0) as i32;
                            }
                        }
                    }
                }
            }
        }
    }

    let free_mem = if max_mem <= 0 {
        8192
    } else {
        std::cmp::max(0, max_mem - total_used_mem)
    };

    let builder_mem_limit = free_mem;
    let builder_mem_request = std::cmp::min(512, builder_mem_limit);

    let build_id = Uuid::new_v4();

    // Immutable image tag: every build pushes its own image so previous images
    // survive (enables rollback) and the layer cache stays coherent.
    let registry_url = std::env::var("HERMES_REGISTRY_URL").unwrap_or_else(|_| "localhost:5000".to_string());
    let full_image_tag = format!("{}/hermes-app-image:{}", registry_url, build_id);
    let builder_pod_name = format!("hermes-builder-{}", instance_id);

    // For Kaniko running inside the cluster, localhost/127.0.0.1 registry must be accessed via the internal registry service
    let mut kaniko_destination = full_image_tag.clone();
    let mut kaniko_registry_host = registry_url.clone();
    if registry_url.contains("localhost") || registry_url.contains("127.0.0.1") {
        kaniko_registry_host = "registry.kube-system.svc.cluster.local:80".to_string();
        kaniko_destination = format!("{}/hermes-app-image:{}", kaniko_registry_host, build_id);
    }
    // Shared layer-cache repository: makes rebuilds that don't change dependencies dramatically faster.
    let kaniko_cache_repo = format!("{}/hermes-build-cache", kaniko_registry_host);

    // Enter the global build queue as 'queued'; it flips to 'building' only once a
    // slot frees up (see the semaphore acquire below).
    let _ = sqlx::query!(
        "INSERT INTO app_builds (id, app_id, app_instance_id, status, phase, logs, commit_message, commit_sha, image_tag) VALUES ($1, $2, $3, 'queued', 'queued', '', NULL, NULL, $4)",
        build_id, meta.app_id, instance_id, full_image_tag
    )
    .execute(&pool)
    .await;

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
            workspace_id: meta.workspace_id,
            build_id,
            app_id: meta.app_id,
            status: "queued".to_string(),
            phase: Some("queued".to_string()),
        }
    );

    // Fetch commit details from GitHub asynchronously (if GitHub repo)
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
                            
                            // Update the record with the fetched commit details
                            let _ = sqlx::query!(
                                "UPDATE app_builds SET commit_message = $1, commit_sha = $2 WHERE id = $3",
                                commit_msg, commit_sha, build_id
                            )
                            .execute(&pool)
                            .await;
                        }
                    }
                }
            }
        }
    }

    // Supersede any older non-finished build for this same instance (queued or
    // building): only the newest build should win. Their loops detect this via phase.
    let _ = sqlx::query!(
        "UPDATE app_builds SET status = 'superseded', phase = 'superseded'
         WHERE app_instance_id = $1 AND id <> $2 AND status IN ('queued', 'building')",
        instance_id, build_id
    )
    .execute(&pool)
    .await;

    // Wait for a build slot (stays 'queued' meanwhile). The permit is held for
    // the rest of the function, releasing automatically when the build ends.
    let _build_permit = build_semaphore().acquire().await;
    // Track this build in the in-progress gauge until it returns (any path).
    let _in_progress = crate::utils::metrics::BuildInProgressGuard::new();

    // While we waited in the queue this build may itself have been cancelled or
    // superseded by an even newer one — bail out cleanly if so.
    if matches!(build_phase_db(&pool, build_id).await.as_deref(), Some("cancelled") | Some("superseded")) {
        return;
    }

    // Slot acquired — promote from the queue to actively building.
    let _ = sqlx::query!(
        "UPDATE app_builds SET status = 'building', phase = 'starting' WHERE id = $1",
        build_id
    )
    .execute(&pool)
    .await;
    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
            workspace_id: meta.workspace_id,
            build_id,
            app_id: meta.app_id,
            status: "building".to_string(),
            phase: Some("starting".to_string()),
        }
    );

    // Set up private registry credentials if configured.
    // Secret names are per-build so concurrent builds can't delete each other's credentials.
    let registry_secret_name = format!("hermes-registry-creds-{}", build_id);
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
            match serde_json::from_value::<k8s_openapi::api::core::v1::Secret>(json!({
                "apiVersion": "v1",
                "kind": "Secret",
                "metadata": {
                    "name": registry_secret_name,
                    "namespace": namespace
                },
                "type": "kubernetes.io/dockerconfigjson",
                "stringData": {
                    ".dockerconfigjson": docker_config_str
                }
            })) {
                Ok(secret_manifest) => {
                    let _ = secrets.patch(
                        &registry_secret_name,
                        &PatchParams::apply("hermes-orchestrator").force(),
                        &Patch::Apply(&secret_manifest)
                    ).await;
                    has_registry_creds = true;
                }
                // Optional creds — don't fail the build, just skip them.
                Err(e) => tracing::warn!("Skipping registry credentials (invalid secret manifest): {}", e),
            }
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
    let ssh_secret_name = format!("hermes-ssh-keys-{}", build_id);

    if has_ssh_keys {
        let mut string_data = serde_json::Map::new();
        for (host, key) in &keys_to_mount {
            let key_name = format!("key-{}", host.replace(":", "_"));
            string_data.insert(key_name, json!(key));
        }

        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
        // SSH keys are mounted into the build pod below, so a bad manifest here must
        // fail the build cleanly (logged) rather than panic the spawned task.
        let secret_manifest: k8s_openapi::api::core::v1::Secret = match serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": ssh_secret_name,
                "namespace": namespace
            },
            "type": "Opaque",
            "stringData": string_data
        })) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("Failed to build SSH keys secret manifest: {}", e);
                let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
                return;
            }
        };

        let _ = secrets.patch(
            &ssh_secret_name,
            &PatchParams::apply("hermes-orchestrator").force(),
            &Patch::Apply(&secret_manifest)
        ).await;
    }

    // Keep the clone URL clean (no token) and keep the token out of the pod's
    // command/spec entirely: it lives in a per-build Secret, mounted into the
    // cloner as the GIT_ACCESS_TOKEN env var, and git reads it via a credential
    // helper. So the token never appears in the build logs nor in `kubectl get pod`.
    let cloner_repo = git_repo.clone();
    let git_token_secret_name = format!("hermes-git-token-{}", build_id);

    // Resolve the clone token + provider credential format. Prefer the app's
    // workspace git credential (multi-provider); fall back to the legacy
    // workspace-creator GitHub token for github.com HTTPS URLs.
    let mut git_token: Option<String> = None;
    let mut cred_user: &str = "x-access-token"; // GitHub format
    let mut cred_host: String = "github.com".to_string();
    if let Some(cred_id) = meta.git_credential_id {
        if let Ok(c) = sqlx::query!(
            "SELECT provider, host, encrypted_token, nonce FROM git_credentials WHERE id = $1",
            cred_id
        ).fetch_one(&pool).await {
            if let Ok(tok) = crate::utils::crypto::decrypt_env_value(&c.encrypted_token, &c.nonce) {
                cred_user = if c.provider == "gitlab" { "oauth2" } else { "x-access-token" };
                cred_host = c.host;
                git_token = Some(tok);
            }
        }
    } else if git_repo.starts_with("https://github.com/") {
        if let Some(ref t) = meta.github_token {
            let t = t.trim();
            if !t.is_empty() { git_token = Some(t.to_string()); }
        }
    }

    let mut has_git_token = false;
    if let Some(ref token) = git_token {
        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
        if let Ok(secret_manifest) = serde_json::from_value::<k8s_openapi::api::core::v1::Secret>(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": { "name": git_token_secret_name, "namespace": namespace },
            "type": "Opaque",
            "stringData": { "token": token }
        })) {
            let _ = secrets.patch(
                &git_token_secret_name,
                &PatchParams::apply("hermes-orchestrator").force(),
                &Patch::Apply(&secret_manifest)
            ).await;
            has_git_token = true;
        }
    }
    let git_cred_setup = if has_git_token {
        format!(
            "if [ -n \"$GIT_ACCESS_TOKEN\" ]; then\n  git config --global credential.helper store\n  printf '%s\\n' \"https://{}:${{GIT_ACCESS_TOKEN}}@{}\" > ~/.git-credentials\n  chmod 600 ~/.git-credentials\nfi\n",
            cred_user, cred_host
        )
    } else {
        String::new()
    };

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

    let go_start = if !start_cmd.is_empty() {
        format!("CMD {}", start_cmd)
    } else {
        "CMD [\"/app/server\"]".to_string()
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

    // Build-time environment: NON-SECRET env vars are baked into the generated
    // Dockerfile as ENV before the install/build steps, so tools like Vite /
    // Next.js / CRA can read VITE_*, NEXT_PUBLIC_*, etc. at `npm run build`.
    // Secret vars are intentionally excluded (they would be layered into the image)
    // and remain runtime-only via the Kubernetes secret.
    let mut build_env_block = String::new();
    for (key, val) in crate::utils::app_env::resolve_instance_build_env(&pool, instance_id).await {
        // Dockerfile ENV is single-line; skip multi-line values.
        if val.contains('\n') || val.contains('\r') {
            continue;
        }
        let escaped = val
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$");
        build_env_block.push_str(&format!("ENV {}=\"{}\"\n", key, escaped));
    }

    let cloner_script = format!(
        r#"set -e
{ssh_setup_script}{git_cred_setup}echo "Clonez {cloner_repo} (branch: {branch_name})..."
if ! git clone --depth 1 --branch {branch_name} {cloner_repo} /workspace; then
  echo "EROARE: git clone a eșuat. Verifică URL-ul repository-ului, branch-ul ({branch_name}) și autentificarea (token GitHub / cheie SSH)." >&2
  exit 1
fi
{change_dir_and_detect}
for f in [dD][oO][cC][kK][eE][rR][fF][iI][lL][eE]; do
  if [ -f "$f" ]; then
    if [ "$f" != "Dockerfile" ]; then
      echo "Found dockerfile with name '$f'. Renaming to 'Dockerfile' for Kaniko compatibility..."
      mv "$f" Dockerfile
    fi
    break
  fi
done
if [ -f Dockerfile ]; then
  echo "Patching Dockerfile paths for correct angular output directory..."
  sed -i 's/dist\/hermes-frontend/dist\/frontend/g' Dockerfile
  echo "Auto-detecting port, env and volumes from the existing Dockerfile..."
  DETECTED_PORT=$(grep -iE '^[[:space:]]*EXPOSE[[:space:]]+[0-9]+' Dockerfile | head -n1 | grep -oE '[0-9]+' | head -n1)
  if [ -n "$DETECTED_PORT" ]; then echo "HERMES_DETECT_PORT=$DETECTED_PORT"; fi
  grep -iE '^[[:space:]]*ENV[[:space:]]' Dockerfile | sed -E 's/^[[:space:]]*[Ee][Nn][Vv][[:space:]]+//' | while IFS= read -r envline; do
    case "$envline" in
      *=*) echo "HERMES_DETECT_ENV=$envline" ;;
      *) ek=$(printf '%s' "$envline" | cut -d' ' -f1); ev=$(printf '%s' "$envline" | cut -d' ' -f2-); echo "HERMES_DETECT_ENV=$ek=$ev" ;;
    esac
  done
  grep -iE '^[[:space:]]*VOLUME[[:space:]]' Dockerfile | sed -E 's/^[[:space:]]*[Vv][Oo][Ll][Uu][Mm][Ee][[:space:]]+//' | while IFS= read -r volline; do
    cleanvol=$(echo "$volline" | tr -d "[]\"'" | tr ',' ' ')
    for v in $cleanvol; do
      if [ -n "$v" ]; then echo "HERMES_DETECT_VOLUME=$v"; fi
    done
  done
fi
if [ ! -f Dockerfile ]; then
  echo "No Dockerfile found, generating fallback..."
  if [ -f index.html ] || [ -f index.htm ]; then
    echo "Detected pure static HTML project. Serving with Nginx..."
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
FROM nginx:alpine
COPY . /usr/share/nginx/html
COPY nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE {internal_port}
CMD ["nginx", "-g", "daemon off;"]
EOF
  elif [ -f package.json ]; then
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
{build_env_block}RUN npm install
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
{build_env_block}RUN npm install
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
{build_env_block}RUN if [ -f requirements.txt ]; then pip install --no-cache-dir -r requirements.txt; fi
{build_instruction}
EXPOSE {internal_port}
{python_start}
EOF
  elif [ -f go.mod ]; then
    echo "Detected Go project"
    cat << 'EOF' > Dockerfile
FROM golang:1.22-alpine AS gobuilder
WORKDIR /app
COPY . .
{build_env_block}RUN go mod download && CGO_ENABLED=0 go build -o /app/server .

FROM alpine:3.19
ENV PORT {internal_port}
WORKDIR /app
COPY --from=gobuilder /app/server /app/server
EXPOSE {internal_port}
{go_start}
EOF
  elif [ -f Cargo.toml ]; then
    echo "Detected Rust project"
    cat << 'EOF' > Dockerfile
FROM rust:1.75
ENV PORT {internal_port}
WORKDIR /app
COPY . .
{build_env_block}{rust_build_instruction}
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
{build_env_block}RUN if [ -f package.json ]; then npm install; fi
{build_instruction}
EXPOSE {internal_port}
{fallback_start}
EOF
  fi
fi"#,
        ssh_setup_script = ssh_setup_script,
        git_cred_setup = git_cred_setup,
        build_env_block = build_env_block,
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
        go_start = go_start,
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
                "image": "alpine/git:2.45.2",
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
                    "--cache=true",
                    format!("--cache-repo={}", kaniko_cache_repo),
                    "--skip-tls-verify",
                    "--insecure",
                    "--insecure-pull"
                ],
                "volumeMounts": [{
                    "name": "workspace",
                    "mountPath": "/workspace"
                }],
                "resources": {
                    "requests": {
                        "cpu": "200m",
                        "memory": format!("{}Mi", builder_mem_request)
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
                            "secretName": registry_secret_name
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

    // Inject the GitHub token into the cloner via a Secret-backed env var so it
    // never appears in the pod's command string / spec.
    if has_git_token {
        if let Some(cloner) = builder_pod_manifest
            .get_mut("spec")
            .and_then(|s| s.get_mut("initContainers"))
            .and_then(|ic| ic.get_mut(0))
        {
            cloner["env"] = json!([{
                "name": "GIT_ACCESS_TOKEN",
                "valueFrom": {
                    "secretKeyRef": { "name": git_token_secret_name, "key": "token" }
                }
            }]);
        }
    }

    let pod_manifest: k8s_openapi::api::core::v1::Pod = match serde_json::from_value(builder_pod_manifest) {
        Ok(p) => p,
        Err(e) => {
            let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
            let error_msg = format!("Eroare la generarea manifestului pod-ului de build: {}", e);
            let duration_sec = start_instant.elapsed().as_secs() as i32;
            let _ = sqlx::query!(
                "UPDATE app_builds SET status = 'failed', phase = 'failed', logs = $1, duration_sec = $2, failure_reason = $4, failure_category = 'MANIFEST' WHERE id = $3",
                error_msg, duration_sec, build_id, "Generarea manifestului pod-ului de build a eșuat."
            )
            .execute(&pool)
            .await;
            crate::utils::metrics::record_build_finished("failed", duration_sec as f64);
            crate::utils::metrics::record_build_failure_category("MANIFEST");

            crate::utils::event_broadcaster::broadcast_event(
                crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
                    workspace_id: meta.workspace_id,
                    build_id,
                    app_id: meta.app_id,
                    status: "failed".to_string(),
                    phase: Some("failed".to_string()),
                }
            );

            return;
        }
    };

    let pods: Api<k8s_openapi::api::core::v1::Pod> = Api::namespaced(k8s_client.clone(), &namespace);

    // The builder pod name is per-instance; clear any leftover pod from a
    // superseded build of the same instance before creating ours.
    let _ = pods.delete(&builder_pod_name, &DeleteParams::default()).await;
    for _ in 0..30 {
        if pods.get(&builder_pod_name).await.is_err() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    if let Err(e) = pods.create(&PostParams::default(), &pod_manifest).await {
        let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
        let error_msg = format!(
            "Eroare la crearea pod-ului de build în Kubernetes (verifică cota de resurse a workspace-ului):\n{}",
            e
        );
        let duration_sec = start_instant.elapsed().as_secs() as i32;
        let _ = sqlx::query!(
            "UPDATE app_builds SET status = 'failed', phase = 'failed', logs = $1, duration_sec = $2, failure_reason = $4, failure_category = 'POD_CREATE' WHERE id = $3",
            error_msg, duration_sec, build_id, "Crearea pod-ului de build a eșuat (probabil cotă de resurse insuficientă în workspace)."
        )
        .execute(&pool)
        .await;
        crate::utils::metrics::record_build_finished("failed", duration_sec as f64);
        crate::utils::metrics::record_build_failure_category("POD_CREATE");

        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
                workspace_id: meta.workspace_id,
                build_id,
                app_id: meta.app_id,
                status: "failed".to_string(),
                phase: Some("failed".to_string()),
            }
        );

        if has_registry_creds {
            let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
            let _ = secrets.delete(&registry_secret_name, &DeleteParams::default()).await;
        }
        if has_ssh_keys {
            let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
            let _ = secrets.delete(&ssh_secret_name, &DeleteParams::default()).await;
        }
        if has_git_token {
            let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
            let _ = secrets.delete(&git_token_secret_name, &DeleteParams::default()).await;
        }
        return;
    }

    // Builder pod created: the init "cloner" container runs first (git clone).
    set_build_phase(&pool, build_id, meta.workspace_id, meta.app_id, "cloning").await;

    let mut success = false;
    let mut cancelled = false;
    let mut timed_out = false;
    let mut building_phase_set = false;
    let timeout = std::time::Duration::from_secs(900); // 15 minutes timeout

    let mut last_pod_status = None;

    loop {
        if start_instant.elapsed() >= timeout {
            timed_out = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Cancellation / supersession are signalled by writing the build's phase.
        if matches!(build_phase_db(&pool, build_id).await.as_deref(), Some("cancelled") | Some("superseded")) {
            cancelled = true;
            break;
        }

        if let Ok(pod) = pods.get(&builder_pod_name).await {
            last_pod_status = pod.status.clone();
            if let Some(ref status) = pod.status {
                // Once the kaniko container starts, the clone is done and the image build is underway.
                if !building_phase_set {
                    let kaniko_started = status.container_statuses.as_ref()
                        .and_then(|cs| cs.iter().find(|c| c.name == "kaniko"))
                        .and_then(|c| c.state.as_ref())
                        .map(|s| s.running.is_some() || s.terminated.is_some())
                        .unwrap_or(false);
                    if kaniko_started {
                        set_build_phase(&pool, build_id, meta.workspace_id, meta.app_id, "building").await;
                        building_phase_set = true;
                    }
                }
                if let Some(ref phase) = status.phase {
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

    // Cancelled or superseded: clean up our own resources and exit without
    // marking the build failed. The pod is owned by whoever initiated the
    // cancellation (the cancel endpoint or the superseding build).
    if cancelled {
        if has_registry_creds {
            let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
            let _ = secrets.delete(&registry_secret_name, &DeleteParams::default()).await;
        }
        if has_ssh_keys {
            let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
            let _ = secrets.delete(&ssh_secret_name, &DeleteParams::default()).await;
        }
        if has_git_token {
            let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
            let _ = secrets.delete(&git_token_secret_name, &DeleteParams::default()).await;
        }
        return;
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
    let mut cloner_exit_code: Option<i32> = None;
    let mut kaniko_exit_code: Option<i32> = None;
    let mut pod_status_reason: Option<String> = None;
    let mut pod_status_message: Option<String> = None;
    let mut kaniko_terminated_reason: Option<String> = None;

    let mut status_to_use = None;
    if let Ok(pod) = pods.get(&builder_pod_name).await {
        status_to_use = pod.status.clone();
    }
    if status_to_use.is_none() {
        status_to_use = last_pod_status;
    }

    if let Some(status) = status_to_use {
        pod_status_reason = status.reason.clone();
        pod_status_message = status.message.clone();
        if let Some(init_statuses) = status.init_container_statuses {
            if let Some(cloner_status) = init_statuses.iter().find(|c| c.name == "cloner") {
                if let Some(ref state) = cloner_status.state {
                    if let Some(ref terminated) = state.terminated {
                        cloner_exit_code = Some(terminated.exit_code);
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
                        kaniko_exit_code = Some(terminated.exit_code);
                        kaniko_terminated_reason = terminated.reason.clone();
                        if let (Some(started), Some(finished)) = (&terminated.started_at, &terminated.finished_at) {
                            let duration = finished.0.signed_duration_since(started.0);
                            kaniko_duration_str = format!("{}s", duration.num_seconds());
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
        let _ = secrets.delete(&registry_secret_name, &DeleteParams::default()).await;
    }

    if has_ssh_keys {
        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = secrets.delete(&ssh_secret_name, &DeleteParams::default()).await;
    }
    if has_git_token {
        let secrets: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = secrets.delete(&git_token_secret_name, &DeleteParams::default()).await;
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

    let status_str = if success { "succeeded" } else if timed_out { "timed_out" } else { "failed" };
    let terminal_phase = if success { "deploying" } else if timed_out { "timed_out" } else { "failed" };
    let (failure_category, failure_reason): (Option<String>, Option<String>) = if success {
        (None, None)
    } else if timed_out {
        (Some("TIMEOUT".to_string()), Some("Build-ul a depășit timpul maxim alocat (15 minute) și a fost oprit automat.".to_string()))
    } else {
        let (cat, reason) = classify_build_failure(
            &cloner_logs,
            &kaniko_logs,
            cloner_exit_code,
            kaniko_exit_code,
            pod_status_reason.as_deref(),
            pod_status_message.as_deref(),
            kaniko_terminated_reason.as_deref(),
        );
        (Some(cat.to_string()), Some(reason))
    };

    if success {
        build_logs.push_str("\n\n=========================================\n");
        build_logs.push_str(&format!(" ETAPA 3: CONSTRUIRE REUȘITĂ (SUCCESS) [Timp Total Build: {}]\n", total_build_duration_str));
        build_logs.push_str("=========================================\n");
        build_logs.push_str("Imaginea Docker a fost creată cu succes și trimisă în registry.\n");
        build_logs.push_str("Se pornește faza de lansare în clusterul Kubernetes...\n");
    } else {
        build_logs.push_str("\n\n=========================================\n");
        let label = if timed_out { "TIMEOUT" } else { "FAILED" };
        build_logs.push_str(&format!(" ETAPA 3: CONSTRUIRE EȘUATĂ ({}) [Timp Total Build: {}]\n", label, total_build_duration_str));
        build_logs.push_str("=========================================\n");
        if let Some(ref cat) = failure_category {
            build_logs.push_str(&format!("Categorie eroare : {}\n", cat));
        }
        if let Some(ref reason) = failure_reason {
            build_logs.push_str(&format!("Diagnostic       : {}\n", reason));
        }
        build_logs.push_str("\nPentru detalii suplimentare, consultă logurile etapelor de mai sus.\n");
    }
    let duration_sec = start_instant.elapsed().as_secs() as i32;

    // Build telemetry as Prometheus metrics (image-build phase outcome only;
    // runtime crashes are tracked separately by the deploy health gate).
    crate::utils::metrics::record_build_finished(status_str, duration_sec as f64);
    if let Some(ref cat) = failure_category {
        crate::utils::metrics::record_build_failure_category(cat);
    }

    let _ = sqlx::query!(
        "UPDATE app_builds SET status = $1, logs = $2, duration_sec = $3, failure_reason = $4, failure_category = $5, phase = $6 WHERE id = $7",
        status_str, build_logs, duration_sec, failure_reason, failure_category, terminal_phase, build_id
    )
    .execute(&pool)
    .await;

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
            workspace_id: meta.workspace_id,
            build_id,
            app_id: meta.app_id,
            status: status_str.to_string(),
            phase: Some(terminal_phase.to_string()),
        }
    );

    if !success {
        let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
        return;
    }

    // Build succeeded — record the freshly built image as the instance's current
    // image so start/redeploy/cron use it instead of reconstructing a stale tag.
    let _ = sqlx::query!(
        "UPDATE app_instances SET current_image_tag = $1 WHERE id = $2",
        full_image_tag, instance_id
    )
    .execute(&pool)
    .await;

    // Auto-configure port and env from the Dockerfile: the cloner emits
    // HERMES_DETECT_PORT / HERMES_DETECT_ENV markers (only for user-provided
    // Dockerfiles) which we read back from the cloner logs here.
    let mut detected_port: Option<i32> = None;
    let mut detected_envs: Vec<(String, String)> = Vec::new();
    let mut detected_volumes: Vec<String> = Vec::new();
    for line in cloner_logs.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("HERMES_DETECT_PORT=") {
            if let Ok(p) = rest.trim().parse::<i32>() {
                if p > 0 && p <= 65535 {
                    detected_port = Some(p);
                }
            }
        } else if let Some(rest) = line.strip_prefix("HERMES_DETECT_ENV=") {
            if let Some((k, v)) = rest.split_once('=') {
                let k = k.trim();
                let valid = !k.is_empty()
                    && k.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
                    && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if valid {
                    detected_envs.push((k.to_string(), v.trim().to_string()));
                }
            }
        } else if let Some(rest) = line.strip_prefix("HERMES_DETECT_VOLUME=") {
            let v = rest.trim().to_string();
            if !v.is_empty() && v.starts_with('/') {
                detected_volumes.push(v);
            }
        }
    }

    // Apply the detected port only when the user hasn't pinned it manually.
    if let Some(port) = detected_port {
        let _ = sqlx::query!(
            "UPDATE app_instances SET internal_port = $1
             WHERE id = $2 AND port_is_auto = true AND internal_port <> $1",
            port, instance_id
        )
        .execute(&pool)
        .await;
    }

    // Import declared ENV defaults as non-secret vars, without overwriting any
    // the user already configured (ON CONFLICT DO NOTHING). Capped to avoid abuse.
    for (key, value) in detected_envs.into_iter().take(50) {
        if let Ok((enc, nonce)) = crate::utils::crypto::encrypt_env_value(&value) {
            let _ = sqlx::query!(
                "INSERT INTO environment_variables (id, workspace_id, app_instance_id, key, encrypted_value, nonce, is_secret)
                 VALUES ($1, $2, $3, $4, $5, $6, false)
                 ON CONFLICT (app_instance_id, key) DO NOTHING",
                Uuid::new_v4(), meta.workspace_id, instance_id, key, enc, nonce
            )
            .execute(&pool)
            .await;
        }
    }

    // Import declared VOLUME mappings as persistent volume records,
    // avoiding duplicates for the same container path (ON CONFLICT / EXISTS checks).
    for vol_path in detected_volumes.into_iter().take(10) {
        let exists = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM app_volumes WHERE app_id = $1 AND container_path = $2)",
            meta.app_id, vol_path
        )
        .fetch_one(&pool)
        .await
        .unwrap_or(Some(false))
        .unwrap_or(false);

        if !exists {
            let volume_id = Uuid::new_v4();
            let host_path = format!("/var/lib/hermes/volumes/{}", volume_id);
            let _ = std::fs::create_dir_all(&host_path);
            
            let _ = sqlx::query!(
                "INSERT INTO app_volumes (id, workspace_id, app_id, name, container_path, host_path)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                volume_id, meta.workspace_id, meta.app_id, format!("auto-{}", &volume_id.to_string()[..8]), vol_path, host_path
            )
            .execute(&pool)
            .await;
            
            tracing::info!(path = %vol_path, "Builder created auto-volume record");
        }
    }

    // Optional vulnerability scan of the pushed image (report-only, runs in
    // parallel with the deploy and appends its findings to the build logs).
    if std::env::var("HERMES_TRIVY_ENABLED").map(|v| v == "true" || v == "1").unwrap_or(false) {
        let pool_scan = pool.clone();
        let client_scan = k8s_client.clone();
        let ns_scan = namespace.clone();
        let image_scan = kaniko_destination.clone();
        tokio::spawn(async move {
            run_trivy_scan(pool_scan, client_scan, ns_scan, image_scan, build_id).await;
        });
    }

    // Move into the deploy phase. update_status() will flip the
    // build to 'running' or 'failed' depending on the deploy outcome.
    set_build_phase(&pool, build_id, meta.workspace_id, meta.app_id, "deploying").await;

    deploy_compiled_app(pool, instance_id, full_image_tag).await;
}

#[tracing::instrument(skip_all, fields(instance_id = %instance_id))]
pub async fn deploy_compiled_app(pool: PgPool, instance_id: Uuid, image_tag: String) {
    let deploy_start_instant = std::time::Instant::now();
    tracing::info!(image = %image_tag, "App deploy started");
    let mut deploy_error: Option<String> = None;
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

        let limits = sqlx::query!("SELECT max_memory_mb, max_storage_gb, max_cpu_millicores FROM workspaces WHERE id = $1", meta.workspace_id)
            .fetch_one(&pool)
            .await;
        let (max_mem, max_storage, max_cpu) = match limits {
            Ok(r) => (r.max_memory_mb, r.max_storage_gb, r.max_cpu_millicores),
            Err(_) => (2048, 10, 0),
        };
        let namespace = format!("hermes-ws-{}", meta.workspace_id);
        let _ = crate::utils::k8s::K8sManager::create_namespace(&k8s_client, &namespace, max_mem, max_storage, max_cpu).await;

        // Effective env = linked project-pool vars + this instance's own vars.
        let envs = crate::utils::app_env::resolve_instance_env(&pool, instance_id).await;

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

            match crate::utils::k8s::K8sManager::deploy_knative_service(
                &k8s_client,
                &namespace,
                app_name,
                &deployment_image,
                envs,
                min_scale,
                max_scale,
                target_concurrency,
                Some(memory_limit_mb as i32),
                None,
            ).await {
                Ok(_) => {
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
                    crate::utils::metrics::record_deploy("app", "success");
                    tracing::info!(duration = %format!("{}s", deploy_start_instant.elapsed().as_secs()), "App deploy succeeded");

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
                Err(e) => {
                    deploy_error = Some(format!("Knative service deployment error: {}", e));
                }
            }
        } else {
            // Cleanup Knative service if transitioning back to standard
            let _ = crate::utils::k8s::K8sManager::delete_knative_service(&k8s_client, &namespace, app_name).await;

            match crate::utils::k8s::K8sManager::deploy_app(
                &k8s_client,
                &namespace,
                app_name,
                &deployment_image,
                meta.internal_port,
                envs,
                binds,
                cpu_limit,
                memory_limit_mb
            ).await {
                Ok(_) => {
                    match crate::utils::k8s::K8sManager::deploy_service(
                        &k8s_client,
                        &namespace,
                        app_name,
                        meta.internal_port
                    ).await {
                        Ok(_) => {
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

                            // Health gate: confirm the pod actually comes up (port
                            // responds) instead of declaring "running" the instant the
                            // manifest is applied. Returns Some(reason) if it crashed.
                            let crash = monitor_deploy_health(&pool, &k8s_client, &namespace, app_name, instance_id, meta.workspace_id, meta.project_id).await;

                            let deploy_duration_str = format!("{}s", deploy_start_instant.elapsed().as_secs());

                            if let Ok(Some(build_rec)) = sqlx::query!(
                                "SELECT id, logs FROM app_builds WHERE app_instance_id = $1 ORDER BY created_at DESC LIMIT 1",
                                instance_id
                            )
                            .fetch_optional(&pool)
                            .await {
                                let mut updated_logs = build_rec.logs;
                                updated_logs.push_str("\n=========================================\n");
                                updated_logs.push_str(&format!(" ETAPA 4: DEPLOY (DEPLOYED) [Durată: {}]\n", deploy_duration_str));
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
                                if let Some(ref reason) = crash {
                                    // Image built & deployed fine, but the container crashed at startup.
                                    updated_logs.push_str(&format!(" ATENȚIE: Build & deploy OK, dar aplicația a crăpat la pornire: {}\n", reason));
                                    updated_logs.push_str(" Imaginea este validă (poți face rollback). Verifică variabilele de mediu și comanda de start.\n");
                                } else {
                                    updated_logs.push_str(" APLICAȚIA A FOST LANSATĂ ȘI ESTE ACTIVĂ!\n");
                                }
                                updated_logs.push_str("=========================================\n");

                                let _ = sqlx::query!(
                                    "UPDATE app_builds SET logs = $1 WHERE id = $2",
                                    updated_logs, build_rec.id
                                )
                                .execute(&pool)
                                .await;
                            }
                            match crash {
                                Some(ref reason) => tracing::warn!(duration = %deploy_duration_str, "App deployed but crashed at startup: {}", reason),
                                None => tracing::info!(duration = %deploy_duration_str, "App deploy succeeded"),
                            }
                            return; // Image built successfully (running or crashed at runtime).
                        }
                        Err(e) => {
                            deploy_error = Some(format!("Service deployment error: {}", e));
                        }
                    }
                }
                Err(e) => {
                    deploy_error = Some(format!("Application deployment error: {}", e));
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
        if let Some(ref err_msg) = deploy_error {
            updated_logs.push_str(&format!("Eroare la provizionarea resurselor Kubernetes în cluster:\n{}\n", err_msg));
        } else {
            updated_logs.push_str("Eroare la provizionarea resurselor Kubernetes în cluster.\n");
        }

        // Only the deploy PHASE failed — the image was built and pushed
        // successfully, so we keep status='succeeded' (the build stays
        // rollback-able and isn't shown as a build failure). The UI surfaces the
        // deploy failure via phase='failed' + failure_reason. This also avoids
        // retroactively marking an older successful build as failed when a
        // reload/rollback deploy fails (deploy_compiled_app is reused for those).
        let _ = sqlx::query!(
            "UPDATE app_builds SET logs = $1, phase = 'failed', failure_reason = $3, failure_category = 'DEPLOY' WHERE id = $2",
            updated_logs, build_rec.id, "Deploy-ul resurselor Kubernetes a eșuat (vezi etapa 4 din log-uri). Imaginea s-a construit corect."
        )
        .execute(&pool)
        .await;
    }

    match deploy_error {
        Some(ref err) => tracing::warn!(duration = %deploy_duration_str, "App deploy failed: {}", err),
        None => tracing::warn!(duration = %deploy_duration_str, "App deploy failed"),
    }
    let _ = update_status(&pool, instance_id, AppStatus::Failed).await;
    crate::utils::metrics::record_deploy("app", "failed");
}

/// Resolve the image tag an instance should run: the immutable tag recorded by
/// its latest successful build, falling back to the legacy per-instance tag for
/// instances built before immutable tags were introduced.
pub async fn resolve_instance_image_tag(pool: &sqlx::PgPool, instance_id: Uuid) -> String {
    let stored = sqlx::query_scalar!(
        "SELECT current_image_tag FROM app_instances WHERE id = $1",
        instance_id
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten();

    stored.unwrap_or_else(|| {
        let registry_url = std::env::var("HERMES_REGISTRY_URL").unwrap_or_else(|_| "localhost:5000".to_string());
        format!("{}/hermes-app-image:{}", registry_url, instance_id)
    })
}

/// Inspect build logs and container exit codes to derive a machine-readable
/// failure category plus an actionable, human-readable explanation.
fn classify_build_failure(
    cloner_logs: &str,
    kaniko_logs: &str,
    cloner_exit_code: Option<i32>,
    kaniko_exit_code: Option<i32>,
    pod_reason: Option<&str>,
    pod_message: Option<&str>,
    kaniko_reason: Option<&str>,
) -> (&'static str, String) {
    let cloner_lower = cloner_logs.to_lowercase();
    let kaniko_lower = kaniko_logs.to_lowercase();

    // --- OOM & Eviction Detection ---
    if kaniko_reason == Some("OOMKilled") || kaniko_exit_code == Some(137) {
        return ("BUILD_OOM", "Build-ul a rămas fără memorie (OOMKilled). Mărește limita de memorie a workspace-ului sau redu consumul comenzii de build.".to_string());
    }

    if pod_reason == Some("Evicted") || pod_message.map(|m| m.to_lowercase().contains("evict")).unwrap_or(false) {
        let msg = pod_message.unwrap_or("Evacuat de Kubernetes din cauza resurselor insuficiente pe nod.");
        return ("BUILD_EVICTED", format!("Pod-ul de build a fost evacuat (Evicted) de Kubernetes. Detaliu: {}", msg));
    }

    // Kaniko duration N/A = containerul nu a înregistrat un timestamp de terminare,
    // semn că podul a fost ucis forțat (OOM la nivel de nod, evicție silențioasă etc.)
    // în timp ce rula o comandă de compilare intensivă.
    if kaniko_exit_code.is_none() && kaniko_reason.is_none() {
        let compiling = kaniko_lower.contains("npm run build")
            || kaniko_lower.contains("ng build")
            || kaniko_lower.contains("cargo build")
            || kaniko_lower.contains("go build")
            || kaniko_lower.contains("webpack")
            || kaniko_lower.contains("vite build");
        let not_done = !kaniko_lower.contains("pushed") && !kaniko_lower.contains("complete");
        if compiling && not_done {
            return ("BUILD_OOM", concat!(
                "Build-ul s-a oprit brusc în timpul compilării (posibil depășire de memorie/OOM la nivel de nod). ",
                "Angular/webpack/bundler-ele necesită de obicei 1-2 GB RAM. ",
                "Verifică că nodul K8s are suficientă memorie liberă sau mărește limita workspace-ului."
            ).to_string());
        }
        // Pod dispărut fără urmă — probabil evicție sau node pressure
        if not_done {
            return ("BUILD_KILLED", concat!(
                "Pod-ul de build a dispărut fără să termine (probabil evicție din cauza presiunii pe resurse). ",
                "Verifică memoria disponibilă pe nodul K8s și încearcă din nou."
            ).to_string());
        }
    }

    // --- Clone stage failures ---
    if cloner_exit_code.map(|c| c != 0).unwrap_or(false) || kaniko_logs.trim().is_empty() {
        if cloner_lower.contains("authentication failed")
            || cloner_lower.contains("could not read username")
            || cloner_lower.contains("permission denied (publickey")
            || cloner_lower.contains("invalid username or password")
        {
            return ("GIT_AUTH", "Autentificarea la repository a eșuat. Reconectează token-ul GitHub sau verifică cheia SSH a proiectului.".to_string());
        }
        if cloner_lower.contains("remote branch")
            || cloner_lower.contains("couldn't find remote ref")
            || cloner_lower.contains("not found in upstream")
        {
            return ("BRANCH_MISSING", "Branch-ul configurat nu există în repository. Verifică numele branch-ului în setările aplicației.".to_string());
        }
        if cloner_lower.contains("repository not found") || cloner_lower.contains("does not exist") {
            return ("REPO_NOT_FOUND", "Repository-ul nu a fost găsit. Verifică URL-ul Git și permisiunile contului.".to_string());
        }
        if cloner_lower.contains("could not resolve host") || cloner_lower.contains("connection timed out") {
            return ("NETWORK", "Eroare de rețea la clonarea codului. Reîncearcă build-ul — de obicei e o problemă temporară.".to_string());
        }
        if cloner_exit_code.map(|c| c != 0).unwrap_or(false) {
            return ("CLONE_FAILED", "Descărcarea codului a eșuat. Consultă log-urile etapei de clonare pentru detalii.".to_string());
        }
    }

    // --- Image build stage failures ---
    if kaniko_lower.contains("error resolving dockerfile path")
        || kaniko_lower.contains("no such file or directory") && kaniko_lower.contains("dockerfile")
    {
        return ("NO_DOCKERFILE", "Nu a fost găsit un Dockerfile în repository. Adaugă un Dockerfile la rădăcină (sau în subcalea configurată).".to_string());
    }
    if kaniko_lower.contains("unauthorized") || kaniko_lower.contains("401") && kaniko_lower.contains("push") {
        return ("REGISTRY_AUTH", "Push-ul imaginii în registry a fost respins (autentificare). Problemă de platformă — verifică credențialele registry-ului.".to_string());
    }
    if kaniko_lower.contains("connection refused") || kaniko_lower.contains("no such host") {
        return ("REGISTRY", "Registry-ul de imagini nu a putut fi contactat. Reîncearcă build-ul; dacă persistă, verifică serviciul de registry.".to_string());
    }
    if kaniko_lower.contains("npm err!")
        || kaniko_lower.contains("error[e")
        || kaniko_lower.contains("syntaxerror")
        || kaniko_lower.contains("compilation failed")
        || kaniko_lower.contains("returned a non-zero code")
        || kaniko_lower.contains("exit code: 1")
    {
        // Pull the last meaningful error lines so the UI can show the cause directly.
        let snippet: Vec<&str> = kaniko_logs
            .lines()
            .filter(|l| {
                let ll = l.to_lowercase();
                ll.contains("error") || ll.contains("err!") || ll.contains("failed")
            })
            .rev()
            .take(3)
            .collect();
        let mut detail = snippet.into_iter().rev().collect::<Vec<_>>().join(" | ");
        if detail.len() > 300 {
            detail.truncate(300);
        }
        let suffix = if detail.is_empty() { String::new() } else { format!(" Detaliu: {}", detail) };
        return ("BUILD_COMMAND", format!("Comanda de build a eșuat în interiorul imaginii.{}", suffix));
    }

    ("UNKNOWN", "Construirea imaginii a eșuat dintr-o cauză neclasificată. Consultă log-urile complete pentru detalii.".to_string())
}

/// Run a one-off Trivy pod that scans the freshly pushed image for HIGH/CRITICAL
/// vulnerabilities and appends the report to the build's logs. Report-only:
/// never blocks or fails the deploy.
async fn run_trivy_scan(
    pool: PgPool,
    k8s_client: kube::Client,
    namespace: String,
    image: String,
    build_id: Uuid,
) {
    use k8s_openapi::api::core::v1::Pod;
    let pods: Api<Pod> = Api::namespaced(k8s_client, &namespace);
    let pod_name = format!("hermes-trivy-{}", &build_id.to_string()[..8]);

    let manifest: Pod = match serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": pod_name, "namespace": namespace },
        "spec": {
            "restartPolicy": "Never",
            "containers": [{
                "name": "trivy",
                "image": "aquasec/trivy:latest",
                "args": ["image", "--insecure", "--severity", "HIGH,CRITICAL", "--no-progress", "--scanners", "vuln", image],
                "resources": {
                    "requests": { "cpu": "100m", "memory": "256Mi" },
                    "limits": { "cpu": "1000m", "memory": "1024Mi" }
                }
            }]
        }
    })) {
        Ok(m) => m,
        Err(_) => return,
    };

    let _ = pods.delete(&pod_name, &DeleteParams::default()).await;
    if pods.create(&PostParams::default(), &manifest).await.is_err() {
        return;
    }

    // Wait up to 5 minutes for the scan to finish (image pull included).
    let mut finished = false;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if let Ok(pod) = pods.get(&pod_name).await {
            if let Some(phase) = pod.status.and_then(|s| s.phase) {
                if phase == "Succeeded" || phase == "Failed" {
                    finished = true;
                    break;
                }
            }
        } else {
            break;
        }
    }

    let mut section = String::from("\n\n=========================================\n ETAPA 3.5: SCANARE SECURITATE (TRIVY)\n=========================================\n");
    if finished {
        match pods.logs(&pod_name, &kube::api::LogParams::default()).await {
            Ok(report) => {
                let criticals = report.matches("CRITICAL").count();
                let highs = report.matches("HIGH").count();
                section.push_str(&format!("Rezumat: ~{} CRITICAL, ~{} HIGH (severități HIGH/CRITICAL raportate)\n\n", criticals, highs));
                // Keep the report bounded so the logs column stays manageable.
                let mut trimmed = report;
                if trimmed.len() > 20_000 {
                    trimmed.truncate(20_000);
                    trimmed.push_str("\n... (raport trunchiat)\n");
                }
                section.push_str(&trimmed);
            }
            Err(_) => section.push_str("Scanarea a rulat dar raportul nu a putut fi citit.\n"),
        }
    } else {
        section.push_str("Scanarea nu s-a încheiat în timpul alocat (5 minute) și a fost abandonată.\n");
    }

    let _ = pods.delete(&pod_name, &DeleteParams::default()).await;

    // Atomic append so we don't clobber the deploy stage's log updates.
    let _ = sqlx::query!(
        "UPDATE app_builds SET logs = logs || $1 WHERE id = $2",
        section, build_id
    )
    .execute(&pool)
    .await;
}

/// Watch a freshly-deployed instance for ~2 minutes: mark it Running once a pod
/// is Ready (port responds), or Crashed on CrashLoopBackOff / image-pull errors,
/// capturing container logs and raising an incident. Falls back to Running if the
/// pod neither becomes ready nor crashes within the window.
/// Returns `Some(short_reason)` if the instance crashed during the watch window,
/// or `None` if it became healthy (or the window elapsed without a crash). The
/// caller uses this to write an accurate ETAPA 4 log section.
async fn monitor_deploy_health(
    pool: &sqlx::PgPool,
    k8s_client: &kube::Client,
    namespace: &str,
    app_name: &str,
    instance_id: Uuid,
    workspace_id: Uuid,
    project_id: Uuid,
) -> Option<String> {
    use k8s_openapi::api::core::v1::Pod;
    let pods: Api<Pod> = Api::namespaced(k8s_client.clone(), namespace);
    let lp = kube::api::ListParams::default().labels(&format!("app={}", app_name));
    let start = std::time::Instant::now();
    let window = std::time::Duration::from_secs(120);

    let crash_reasons = [
        "CrashLoopBackOff", "ImagePullBackOff", "ErrImagePull",
        "CreateContainerError", "CreateContainerConfigError", "RunContainerError", "InvalidImageName",
    ];

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let mut healthy = false;
        let mut crash: Option<(String, String)> = None; // (reason, pod_name)

        if let Ok(list) = pods.list(&lp).await {
            for pod in list.items {
                let pod_name = pod.metadata.name.clone().unwrap_or_default();
                if let Some(status) = pod.status {
                    if let Some(ref conds) = status.conditions {
                        if conds.iter().any(|c| c.type_ == "Ready" && c.status == "True") {
                            healthy = true;
                        }
                    }
                    if let Some(ref cs) = status.container_statuses {
                        for c in cs {
                            if let Some(ref state) = c.state {
                                if let Some(ref waiting) = state.waiting {
                                    if let Some(ref reason) = waiting.reason {
                                        if crash_reasons.contains(&reason.as_str()) {
                                            crash = Some((reason.clone(), pod_name.clone()));
                                        }
                                    }
                                }
                            }
                            if c.restart_count >= 3 {
                                crash = Some(("CrashLoopBackOff".to_string(), pod_name.clone()));
                            }
                        }
                    }
                }
            }
        }

        if healthy {
            let _ = update_status(pool, instance_id, AppStatus::Running).await;
            return None;
        }

        if let Some((reason, pod_name)) = crash {
            let log_params = kube::api::LogParams { tail_lines: Some(50), ..Default::default() };
            let container_logs = pods.logs(&pod_name, &log_params).await.unwrap_or_else(|_| "(log-urile containerului nu au putut fi citite)".to_string());

            let _ = update_status(pool, instance_id, AppStatus::Crashed).await;

            let short_reason = match reason.as_str() {
                "ImagePullBackOff" | "ErrImagePull" | "InvalidImageName" => "Imaginea nu a putut fi descărcată/rezolvată".to_string(),
                "CreateContainerError" | "CreateContainerConfigError" | "RunContainerError" => "Containerul nu a putut porni (configurare invalidă)".to_string(),
                _ => "Aplicația a crăpat la pornire (restart în buclă)".to_string(),
            };

            // Mark the deploy phase as crashed without touching the build status:
            // the image was built and pushed successfully — the crash is a runtime
            // issue (wrong env vars, missing port, bad start command, etc.), not a
            // build failure. Keeping status='succeeded' lets the UI distinguish
            // "build OK, app crashed at startup" from "Kaniko/git failed".
            let _ = sqlx::query!(
                "UPDATE app_builds
                 SET phase = 'crashed', failure_category = 'CRASH', failure_reason = $1
                 WHERE id = (SELECT id FROM app_builds WHERE app_instance_id = $2 ORDER BY created_at DESC LIMIT 1)",
                format!("Build reușit, dar aplicația a crăpat la pornire: {} ({}). Verifică variabilele de mediu și comanda de start.", short_reason, reason),
                instance_id
            )
            .execute(pool)
            .await;

            // Raise an incident with the captured logs.
            let incident_id = Uuid::new_v4();
            let message = format!("{} ({}).\n\nUltimele linii din container:\n{}", short_reason, reason, container_logs);
            let _ = sqlx::query!(
                "INSERT INTO app_incident_logs (id, workspace_id, app_instance_id, incident_type, message) VALUES ($1, $2, $3, $4, $5)",
                incident_id, workspace_id, instance_id, format!("crash:{}", reason), message
            )
            .execute(pool)
            .await;

            crate::utils::event_broadcaster::broadcast_event(
                crate::utils::event_broadcaster::SystemEvent::IncidentCreated {
                    workspace_id,
                    incident_id,
                    project_id,
                    message: short_reason.clone(),
                }
            );
            return Some(short_reason);
        }

        if start.elapsed() >= window {
            // Deployed but never confirmed ready and never crashed within the window:
            // assume a slow start and leave it Running rather than block forever.
            let _ = update_status(pool, instance_id, AppStatus::Running).await;
            return None;
        }
    }
}

/// Update a build's granular lifecycle phase and broadcast it live.
async fn set_build_phase(pool: &sqlx::PgPool, build_id: Uuid, workspace_id: Uuid, app_id: Uuid, phase: &str) {
    let _ = sqlx::query!("UPDATE app_builds SET phase = $1 WHERE id = $2", phase, build_id)
        .execute(pool)
        .await;
    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
            workspace_id,
            build_id,
            app_id,
            status: "building".to_string(),
            phase: Some(phase.to_string()),
        }
    );
}

async fn update_status(pool: &sqlx::PgPool, id: Uuid, status: AppStatus) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE app_instances SET status = $1, updated_at = now() WHERE id = $2", status.clone() as AppStatus, id)
        .execute(pool)
        .await?;

    // Reflect terminal deploy outcomes onto the instance's most recent build phase,
    // so the build stepper reaches 'running' or 'failed' from any deploy path.
    let terminal_phase = match status {
        AppStatus::Running => Some("running"),
        AppStatus::Failed => Some("failed"),
        // Crashed = runtime crash after a successful build; monitor_deploy_health
        // already updated the build phase directly with a clear message.
        AppStatus::Crashed => None,
        _ => None,
    };
    if let Some(phase) = terminal_phase {
        let _ = sqlx::query!(
            "UPDATE app_builds SET phase = $1
             WHERE id = (SELECT id FROM app_builds WHERE app_instance_id = $2 ORDER BY created_at DESC LIMIT 1)
               AND phase NOT IN ('cancelled', 'superseded', 'timed_out')",
            phase, id
        )
        .execute(pool)
        .await;
    }

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