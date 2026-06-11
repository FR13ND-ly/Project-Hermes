use axum::{extract::{State, Path, Query}, Json};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;
use crate::dtos::auth_dto::UserResponse;
use crate::models::user_model::User;

#[derive(Debug, Deserialize)]
pub struct LinkGithubRequest {
    pub token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GithubUserResponse {
    pub login: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GithubRepoOwner {
    pub login: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GithubRepo {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub owner: GithubRepoOwner,
    pub private: bool,
    pub html_url: String,
    pub description: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GithubBranch {
    pub name: String,
}

pub async fn link_github_token(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<LinkGithubRequest>,
) -> Result<Json<UserResponse>, AppError> {
    let (token, username) = match payload.token {
        Some(ref t) if !t.trim().is_empty() => {
            let t = t.trim();
            // Verify token with GitHub
            let client = reqwest::Client::new();
            let res = client.get("https://api.github.com/user")
                .header("Authorization", format!("Bearer {}", t))
                .header("User-Agent", "hermes-orchestrator")
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .map_err(|e| AppError::Validation(format!("GitHub API network error: {}", e)))?;

            if !res.status().is_success() {
                return Err(AppError::Validation("Invalid GitHub personal access token".to_string()));
            }

            let github_user = res.json::<GithubUserResponse>()
                .await
                .map_err(|_| AppError::Validation("Failed to parse user profile from GitHub".to_string()))?;

            (Some(t.to_string()), Some(github_user.login))
        }
        _ => (None, None),
    };

    sqlx::query!(
        "UPDATE users SET github_token = $1, github_username = $2, updated_at = now() WHERE id = $3",
        token,
        username,
        claims.sub
    )
    .execute(&state.pool)
    .await?;

    let updated_user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(claims.sub)
        .fetch_one(&state.pool)
        .await?;

    Ok(Json(UserResponse::from(updated_user)))
}

pub async fn list_github_repos(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<Vec<GithubRepo>>, AppError> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(claims.sub)
        .fetch_one(&state.pool)
        .await?;

    let token = match user.github_token {
        Some(t) if !t.is_empty() => t,
        _ => return Err(AppError::Validation("GitHub account not linked".to_string())),
    };

    let client = reqwest::Client::new();
    let res = client.get("https://api.github.com/user/repos?per_page=100&sort=updated")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "hermes-orchestrator")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| AppError::Fatal(e.into()))?;

    if !res.status().is_success() {
        return Err(AppError::Validation("Failed to fetch repositories from GitHub".to_string()));
    }

    let repos = res.json::<Vec<GithubRepo>>()
        .await
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to parse GitHub repository response: {}", e)))?;

    Ok(Json(repos))
}

pub async fn list_github_branches(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<Vec<GithubBranch>>, AppError> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(claims.sub)
        .fetch_one(&state.pool)
        .await?;

    let token = match user.github_token {
        Some(t) if !t.is_empty() => t,
        _ => return Err(AppError::Validation("GitHub account not linked".to_string())),
    };

    let client = reqwest::Client::new();
    let url = format!("https://api.github.com/repos/{}/{}/branches?per_page=100", owner, repo);
    let res = client.get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "hermes-orchestrator")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| AppError::Fatal(e.into()))?;

    if !res.status().is_success() {
        return Err(AppError::Validation("Failed to fetch branches from GitHub".to_string()));
    }

    let branches = res.json::<Vec<GithubBranch>>()
        .await
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to parse GitHub branches response: {}", e)))?;

    Ok(Json(branches))
}

#[derive(Debug, Deserialize)]
pub struct GithubContentItem {
    pub name: String,
    pub r#type: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DetectedEnvVar {
    pub key: String,
    pub value: String,
}

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

#[derive(Debug, Deserialize)]
pub struct DetectQuery {
    pub path: Option<String>,
}

pub async fn detect_project_type(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<DetectQuery>,
) -> Result<Json<ProjectDetectionResponse>, AppError> {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(claims.sub)
        .fetch_one(&state.pool)
        .await?;

    let token = match user.github_token {
        Some(t) if !t.is_empty() => t,
        _ => return Err(AppError::Validation("GitHub account not linked".to_string())),
    };

    let client = reqwest::Client::new();
    let url = if let Some(ref p) = query.path {
        let clean_path = p.trim().trim_start_matches('/').trim_end_matches('/');
        if !clean_path.is_empty() {
            format!("https://api.github.com/repos/{}/{}/contents/{}", owner, repo, clean_path)
        } else {
            format!("https://api.github.com/repos/{}/{}/contents", owner, repo)
        }
    } else {
        format!("https://api.github.com/repos/{}/{}/contents", owner, repo)
    };
    let res = client.get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "hermes-orchestrator")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| AppError::Fatal(e.into()))?;

    if !res.status().is_success() {
        return Err(AppError::Validation("Failed to read repository contents from GitHub".to_string()));
    }

    let items = res.json::<Vec<GithubContentItem>>()
        .await
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to parse GitHub contents response: {}", e)))?;

    let has_file = |name: &str| -> bool {
        items.iter().any(|item| item.name.to_lowercase() == name.to_lowercase() && item.r#type == "file")
    };

    // 1. Scan candidate subdirectories if we are at root
    let is_root = query.path.as_deref().map(|s| s.trim().trim_matches('/')).unwrap_or("").is_empty();
    let mut subdirectories = Vec::new();
    if is_root {
        let dirs: Vec<String> = items.iter()
            .filter(|item| item.r#type == "dir" && !item.name.starts_with('.') && item.name != "node_modules" && item.name != "target" && item.name != "dist" && item.name != "build")
            .map(|item| item.name.clone())
            .collect();
        
        let mut futures = Vec::new();
        for d in &dirs {
            let client_clone = client.clone();
            let token_clone = token.clone();
            let owner_clone = owner.clone();
            let repo_clone = repo.clone();
            let d_clone = d.clone();
            futures.push(tokio::spawn(async move {
                let url = format!("https://api.github.com/repos/{}/{}/contents/{}", owner_clone, repo_clone, d_clone);
                if let Ok(res) = client_clone.get(&url)
                    .header("Authorization", format!("Bearer {}", token_clone))
                    .header("User-Agent", "hermes-orchestrator")
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                {
                    if res.status().is_success() {
                        if let Ok(sub_items) = res.json::<Vec<GithubContentItem>>().await {
                            let has_proj_file = sub_items.iter().any(|item| {
                                let n = item.name.to_lowercase();
                                (n == "dockerfile" || n == "package.json" || n == "requirements.txt" || n == "cargo.toml" || n == "go.mod" || n == "index.html" || n == "index.htm") && item.r#type == "file"
                            });
                            if has_proj_file {
                                return Some(d_clone);
                            }
                        }
                    }
                }
                None
            }));
        }
        
        let results = futures_util::future::join_all(futures).await;
        for r in results {
            if let Ok(Some(valid_dir)) = r {
                subdirectories.push(valid_dir);
            }
        }
        subdirectories.sort();
    }

    // 2. Parse Dockerfile and Env Variables
    let mut detected_envs = Vec::new();
    let mut detected_port: Option<i32> = None;
    let target_path = query.path.as_deref().map(|s| s.trim().trim_start_matches('/').trim_end_matches('/')).unwrap_or("");

    if has_file("Dockerfile") {
        let dockerfile_path = if target_path.is_empty() {
            "Dockerfile".to_string()
        } else {
            format!("{}/Dockerfile", target_path)
        };
        
        let df_url = format!("https://api.github.com/repos/{}/{}/contents/{}", owner, repo, dockerfile_path);
        if let Ok(df_res) = client.get(&df_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "hermes-orchestrator")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            #[derive(Debug, Deserialize)]
            struct GithubContentFile {
                content: String,
                encoding: String,
            }
            if df_res.status().is_success() {
                if let Ok(file_data) = df_res.json::<GithubContentFile>().await {
                    if file_data.encoding == "base64" {
                        let cleaned_content = file_data.content.replace("\n", "").replace("\r", "");
                        if let Ok(decoded_bytes) = BASE64.decode(cleaned_content) {
                            if let Ok(content_str) = String::from_utf8(decoded_bytes) {
                                for line in content_str.lines() {
                                    let trimmed = line.trim();
                                    if trimmed.to_uppercase().starts_with("EXPOSE ") {
                                        let port_body = trimmed["EXPOSE ".len()..].trim();
                                        if let Some(first_port) = port_body.split_whitespace().next() {
                                            if let Ok(port) = first_port.parse::<i32>() {
                                                detected_port = Some(port);
                                            }
                                        }
                                    } else if trimmed.to_uppercase().starts_with("ENV ") {
                                        let env_body = trimmed["ENV ".len()..].trim();
                                        if let Some(eq_idx) = env_body.find('=') {
                                            let key = env_body[..eq_idx].trim().to_string();
                                            let val = env_body[eq_idx + 1..].trim().trim_matches('"').trim_matches('\'').to_string();
                                            if !key.is_empty() {
                                                detected_envs.push(DetectedEnvVar { key, value: val });
                                            }
                                        } else {
                                            let parts: Vec<&str> = env_body.split_whitespace().collect();
                                            if parts.len() >= 2 {
                                                let key = parts[0].to_string();
                                                let val = parts[1..].join(" ").trim_matches('"').trim_matches('\'').to_string();
                                                detected_envs.push(DetectedEnvVar { key, value: val });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let env_file_name = if has_file(".env.example") {
        Some(".env.example")
    } else if has_file(".env") {
        Some(".env")
    } else {
        None
    };

    if let Some(env_name) = env_file_name {
        let env_file_path = if target_path.is_empty() {
            env_name.to_string()
        } else {
            format!("{}/{}", target_path, env_name)
        };
        
        let env_url = format!("https://api.github.com/repos/{}/{}/contents/{}", owner, repo, env_file_path);
        if let Ok(env_res) = client.get(&env_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "hermes-orchestrator")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            #[derive(Debug, Deserialize)]
            struct GithubContentFile {
                content: String,
                encoding: String,
            }
            if env_res.status().is_success() {
                if let Ok(file_data) = env_res.json::<GithubContentFile>().await {
                    if file_data.encoding == "base64" {
                        let cleaned_content = file_data.content.replace("\n", "").replace("\r", "");
                        if let Ok(decoded_bytes) = BASE64.decode(cleaned_content) {
                            if let Ok(content_str) = String::from_utf8(decoded_bytes) {
                                for line in content_str.lines() {
                                    let trimmed = line.trim();
                                    if trimmed.is_empty() || trimmed.starts_with('#') {
                                        continue;
                                    }
                                    if let Some(eq_idx) = trimmed.find('=') {
                                        let key = trimmed[..eq_idx].trim().to_string();
                                        let val = trimmed[eq_idx + 1..].trim().trim_matches('"').trim_matches('\'').to_string();
                                        if !key.is_empty() {
                                            if !detected_envs.iter().any(|e| e.key == key) {
                                                detected_envs.push(DetectedEnvVar { key, value: val });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut detection = ProjectDetectionResponse {
        project_type: "generic".to_string(),
        build_command: "".to_string(),
        start_command: "".to_string(),
        internal_port: detected_port.unwrap_or(8080),
        description: "Tip proiect nespecificat. Introduceți comenzi personalizate de build și start.".to_string(),
        detected_envs,
        subdirectories,
    };

    if has_file("Dockerfile") {
        detection.project_type = "dockerfile".to_string();
        if detected_port.is_none() {
            detection.internal_port = 8080;
        }
        detection.description = "Dockerfile detectat în folderul specificat. Hermes va construi imaginea folosind acest Dockerfile.".to_string();
    } else if has_file("package.json") {
        detection.project_type = "nodejs".to_string();
        detection.build_command = "npm run build".to_string();
        detection.start_command = "npm start".to_string();
        detection.internal_port = detected_port.unwrap_or(3000);
        detection.description = "Proiect Node.js detectat.".to_string();

        let package_json_path = if target_path.is_empty() {
            "package.json".to_string()
        } else {
            format!("{}/package.json", target_path)
        };
        let p_url = format!("https://api.github.com/repos/{}/{}/contents/{}", owner, repo, package_json_path);
        if let Ok(p_res) = client.get(&p_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "hermes-orchestrator")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            #[derive(Debug, Deserialize)]
            struct GithubContentFile {
                content: String,
                encoding: String,
            }
            if p_res.status().is_success() {
                if let Ok(file_data) = p_res.json::<GithubContentFile>().await {
                    if file_data.encoding == "base64" {
                        let cleaned_content = file_data.content.replace("\n", "").replace("\r", "");
                        if let Ok(decoded_bytes) = BASE64.decode(cleaned_content) {
                            if let Ok(package_json_str) = String::from_utf8(decoded_bytes) {
                                if let Ok(package_json) = serde_json::from_str::<serde_json::Value>(&package_json_str) {
                                    let deps = package_json.get("dependencies");
                                    let dev_deps = package_json.get("devDependencies");
                                    
                                    let has_dep = |name: &str| -> bool {
                                        let check = |d: &serde_json::Value| d.get(name).is_some();
                                        deps.map(check).unwrap_or(false) || dev_deps.map(check).unwrap_or(false)
                                    };

                                    if has_dep("next") {
                                        detection.project_type = "nextjs".to_string();
                                        detection.build_command = "npm run build".to_string();
                                        detection.start_command = "npm start".to_string();
                                        detection.internal_port = detected_port.unwrap_or(3000);
                                        detection.description = "Proiect Next.js detectat.".to_string();
                                    } else if has_dep("@angular/core") {
                                        detection.project_type = "angular".to_string();
                                        detection.build_command = "npm run build".to_string();
                                        detection.start_command = "npm start".to_string();
                                        detection.internal_port = detected_port.unwrap_or(4200);
                                        detection.description = "Proiect Angular detectat.".to_string();
                                    } else if has_dep("react") {
                                        detection.project_type = "react".to_string();
                                        detection.build_command = "npm run build".to_string();
                                        detection.start_command = "npm run preview".to_string();
                                        detection.internal_port = detected_port.unwrap_or(4173);
                                        detection.description = "Proiect React detectat.".to_string();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    } else if has_file("requirements.txt") || has_file("Pipfile") || has_file("main.py") {
        detection.project_type = "python".to_string();
        detection.build_command = "pip install -r requirements.txt".to_string();
        detection.start_command = "uvicorn main:app --host 0.0.0.0 --port 8000".to_string();
        detection.internal_port = detected_port.unwrap_or(8000);
        detection.description = "Proiect Python detectat.".to_string();
    } else if has_file("Cargo.toml") {
        detection.project_type = "rust".to_string();
        detection.build_command = "cargo build --release".to_string();
        detection.start_command = format!("./target/release/{}", repo);
        detection.internal_port = detected_port.unwrap_or(8080);
        detection.description = "Proiect Rust detectat.".to_string();
    }

    Ok(Json(detection))
}
