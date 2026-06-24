use axum::{extract::{State, Path, Query}, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::models::git_credential_model::GitCredential;
use crate::dtos::git_credential_dto::{CreateGitCredentialRequest, GitCredentialResponse, GitRepoResponse};
use crate::utils::{crypto, error::AppError};
use crate::utils::git_provider::GitProviderKind;

fn to_cred_response(c: GitCredential) -> GitCredentialResponse {
    GitCredentialResponse {
        id: c.id,
        provider: c.provider,
        host: c.host,
        label: c.label,
        username: c.username,
        created_at: c.created_at,
        skip_tls_verify: c.skip_tls_verify,
    }
}

/// Load a workspace credential and return (provider, host, decrypted token, skip_tls_verify).
async fn load_credential(pool: &sqlx::PgPool, id: Uuid, ws_id: Uuid) -> Result<(GitProviderKind, String, String, bool), AppError> {
    let c = sqlx::query_as::<_, GitCredential>(
        "SELECT * FROM git_credentials WHERE id = $1 AND workspace_id = $2"
    )
    .bind(id)
    .bind(ws_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Git credential not found in this workspace.".to_string()))?;
    let kind = GitProviderKind::parse(&c.provider)?;
    let token = crypto::decrypt_env_value(&c.encrypted_token, &c.nonce)?;
    Ok((kind, c.host, token, c.skip_tls_verify))
}

// --- Credentials CRUD ---

pub async fn list_credentials(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<Vec<GitCredentialResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let creds = sqlx::query_as::<_, GitCredential>(
        "SELECT * FROM git_credentials WHERE workspace_id = $1 ORDER BY created_at DESC"
    )
    .bind(ws_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(creds.into_iter().map(to_cred_response).collect()))
}

pub async fn create_credential(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateGitCredentialRequest>,
) -> Result<Json<GitCredentialResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;

    let kind = GitProviderKind::parse(&payload.provider)?;
    let host = payload.host.as_deref().map(str::trim).filter(|s| !s.is_empty())
        .unwrap_or(kind.default_host()).to_string();
    let label = payload.label.trim();
    if label.is_empty() {
        return Err(AppError::Validation("The credential label is required.".to_string()));
    }
    let token = payload.token.trim();
    if token.is_empty() {
        return Err(AppError::Validation("The token is required.".to_string()));
    }
    let skip_tls = payload.skip_tls_verify.unwrap_or(false);

    // Verify the token against the provider and capture the account login.
    let username = kind.verify(&host, token, skip_tls).await?;

    let (encrypted_token, nonce) = crypto::encrypt_env_value(token)?;
    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO git_credentials (id, workspace_id, provider, host, label, username, encrypted_token, nonce, created_by, skip_tls_verify)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        id, ws_id, kind.as_str(), host, label, username, encrypted_token, nonce, claims.sub, skip_tls
    )
    .execute(&state.pool)
    .await?;

    let c = sqlx::query_as::<_, GitCredential>("SELECT * FROM git_credentials WHERE id = $1")
        .bind(id).fetch_one(&state.pool).await?;
    Ok(Json(to_cred_response(c)))
}

pub async fn delete_credential(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let deleted = sqlx::query!(
        "DELETE FROM git_credentials WHERE id = $1 AND workspace_id = $2", id, ws_id
    )
    .execute(&state.pool)
    .await?
    .rows_affected();
    if deleted == 0 {
        return Err(AppError::NotFound("Git credential not found.".to_string()));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- Repo browsing (credential-scoped) ---

#[derive(Debug, Deserialize)]
pub struct RepoQuery {
    pub repo: String,
    pub path: Option<String>,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
}

pub async fn list_repos(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(cred_id): Path<Uuid>,
) -> Result<Json<Vec<GitRepoResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let (kind, host, token, skip_tls) = load_credential(&state.pool, cred_id, ws_id).await?;
    let repos = kind.list_repos(&host, &token, skip_tls).await?;
    Ok(Json(repos.into_iter().map(|r| GitRepoResponse {
        full_path: r.full_path, name: r.name, private: r.private,
        default_branch: r.default_branch, html_url: r.html_url,
    }).collect()))
}

pub async fn list_branches(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(cred_id): Path<Uuid>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<Vec<String>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let (kind, host, token, skip_tls) = load_credential(&state.pool, cred_id, ws_id).await?;
    Ok(Json(kind.list_branches(&host, &token, &q.repo, skip_tls).await?))
}

// --- Provider-agnostic project detection ---

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DetectedEnvVar { pub key: String, pub value: String }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetectionResponse {
    pub project_type: String,
    pub build_command: String,
    pub start_command: String,
    pub internal_port: i32,
    pub description: String,
    pub detected_envs: Vec<DetectedEnvVar>,
    pub subdirectories: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeFileResponse {
    pub found: bool,
    pub filename: Option<String>,
    pub yaml: String,
}

fn parse_dockerfile(content: &str, envs: &mut Vec<DetectedEnvVar>) -> Option<i32> {
    let mut port = None;
    for line in content.lines() {
        let t = line.trim();
        let up = t.to_uppercase();
        if up.starts_with("EXPOSE ") {
            if let Some(first) = t["EXPOSE ".len()..].trim().split_whitespace().next() {
                if let Ok(p) = first.parse::<i32>() { port = Some(p); }
            }
        } else if up.starts_with("ENV ") {
            let body = t["ENV ".len()..].trim();
            if let Some(eq) = body.find('=') {
                let key = body[..eq].trim().to_string();
                let val = body[eq + 1..].trim().trim_matches('"').trim_matches('\'').to_string();
                if !key.is_empty() { envs.push(DetectedEnvVar { key, value: val }); }
            } else {
                let parts: Vec<&str> = body.split_whitespace().collect();
                if parts.len() >= 2 {
                    envs.push(DetectedEnvVar { key: parts[0].to_string(), value: parts[1..].join(" ").trim_matches('"').trim_matches('\'').to_string() });
                }
            }
        }
    }
    port
}

fn parse_env_file(content: &str, envs: &mut Vec<DetectedEnvVar>) {
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') { continue; }
        if let Some(eq) = t.find('=') {
            let key = t[..eq].trim().to_string();
            let val = t[eq + 1..].trim().trim_matches('"').trim_matches('\'').to_string();
            if !key.is_empty() && !envs.iter().any(|e| e.key == key) {
                envs.push(DetectedEnvVar { key, value: val });
            }
        }
    }
}

pub async fn detect_project(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(cred_id): Path<Uuid>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<ProjectDetectionResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let (kind, host, token, skip_tls) = load_credential(&state.pool, cred_id, ws_id).await?;
    let git_ref = q.git_ref.as_deref();
    let path = q.path.as_deref().map(|s| s.trim().trim_matches('/')).unwrap_or("");

    let items = kind.list_dir(&host, &token, &q.repo, path, git_ref, skip_tls).await?;
    let has_file = |name: &str| items.iter().any(|i| !i.is_dir && i.name.eq_ignore_ascii_case(name));

    // Subdirectories that look like deployable projects (only at root).
    let mut subdirectories = Vec::new();
    if path.is_empty() {
        let dirs: Vec<String> = items.iter()
            .filter(|i| i.is_dir && !i.name.starts_with('.') && !["node_modules", "target", "dist", "build"].contains(&i.name.as_str()))
            .map(|i| i.name.clone())
            .collect();
        for d in dirs {
            if let Ok(sub) = kind.list_dir(&host, &token, &q.repo, &d, git_ref, skip_tls).await {
                let looks_like_proj = sub.iter().any(|i| {
                    let n = i.name.to_lowercase();
                    !i.is_dir && matches!(n.as_str(), "dockerfile" | "package.json" | "requirements.txt" | "cargo.toml" | "go.mod" | "index.html" | "index.htm")
                });
                if looks_like_proj { subdirectories.push(d); }
            }
        }
        subdirectories.sort();
    }

    let join = |name: &str| if path.is_empty() { name.to_string() } else { format!("{}/{}", path, name) };

    let mut detected_envs = Vec::new();
    let mut detected_port: Option<i32> = None;

    if has_file("Dockerfile") {
        if let Ok(Some(content)) = kind.read_file(&host, &token, &q.repo, &join("Dockerfile"), git_ref, skip_tls).await {
            detected_port = parse_dockerfile(&content, &mut detected_envs);
        }
    }
    for env_name in [".env.example", ".env"] {
        if has_file(env_name) {
            if let Ok(Some(content)) = kind.read_file(&host, &token, &q.repo, &join(env_name), git_ref, skip_tls).await {
                parse_env_file(&content, &mut detected_envs);
            }
            break;
        }
    }

    let mut detection = ProjectDetectionResponse {
        project_type: "generic".to_string(),
        build_command: String::new(),
        start_command: String::new(),
        internal_port: detected_port.unwrap_or(8080),
        description: "Project type unspecified. Enter custom build and start commands.".to_string(),
        detected_envs,
        subdirectories,
    };

    if has_file("Dockerfile") {
        detection.project_type = "dockerfile".to_string();
        detection.description = "Dockerfile detectat. Hermes va construi imaginea folosind acest Dockerfile.".to_string();
    } else if has_file("package.json") {
        detection.project_type = "nodejs".to_string();
        detection.build_command = "npm run build".to_string();
        detection.start_command = "npm start".to_string();
        detection.internal_port = detected_port.unwrap_or(3000);
        detection.description = "Node.js project detected.".to_string();
        if let Ok(Some(pkg)) = kind.read_file(&host, &token, &q.repo, &join("package.json"), git_ref, skip_tls).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&pkg) {
                let has_dep = |name: &str| {
                    let check = |d: &serde_json::Value| d.get(name).is_some();
                    json.get("dependencies").map(check).unwrap_or(false) || json.get("devDependencies").map(check).unwrap_or(false)
                };
                if has_dep("next") {
                    detection.project_type = "nextjs".to_string();
                    detection.internal_port = detected_port.unwrap_or(3000);
                    detection.description = "Next.js project detected.".to_string();
                } else if has_dep("@angular/core") {
                    detection.project_type = "angular".to_string();
                    detection.internal_port = detected_port.unwrap_or(4200);
                    detection.description = "Angular project detected.".to_string();
                } else if has_dep("react") {
                    detection.project_type = "react".to_string();
                    detection.start_command = "npm run preview".to_string();
                    detection.internal_port = detected_port.unwrap_or(4173);
                    detection.description = "React project detected.".to_string();
                }
            }
        }
    } else if has_file("requirements.txt") || has_file("Pipfile") || has_file("main.py") {
        detection.project_type = "python".to_string();
        detection.build_command = "pip install -r requirements.txt".to_string();
        detection.start_command = "uvicorn main:app --host 0.0.0.0 --port 8000".to_string();
        detection.internal_port = detected_port.unwrap_or(8000);
        detection.description = "Python project detected.".to_string();
    } else if has_file("Cargo.toml") {
        detection.project_type = "rust".to_string();
        detection.build_command = "cargo build --release".to_string();
        detection.internal_port = detected_port.unwrap_or(8080);
        detection.description = "Rust project detected.".to_string();
    }

    Ok(Json(detection))
}

pub async fn get_compose(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(cred_id): Path<Uuid>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<ComposeFileResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let (kind, host, token, skip_tls) = load_credential(&state.pool, cred_id, ws_id).await?;
    let git_ref = q.git_ref.as_deref();
    let prefix = q.path.as_deref().map(|s| s.trim().trim_matches('/')).filter(|s| !s.is_empty());

    for name in ["docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"] {
        let p = match prefix { Some(pre) => format!("{}/{}", pre, name), None => name.to_string() };
        if let Ok(Some(yaml)) = kind.read_file(&host, &token, &q.repo, &p, git_ref, skip_tls).await {
            return Ok(Json(ComposeFileResponse { found: true, filename: Some(name.to_string()), yaml }));
        }
    }
    Ok(Json(ComposeFileResponse { found: false, filename: None, yaml: String::new() }))
}
