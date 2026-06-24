//! Provider-agnostic git host access (GitHub, GitLab). Powers credential
//! verification, the repo/branch pickers, and provider-agnostic project detection.
//! Adding a provider = one more arm in `GitProviderKind` and its match arms.
//! (All user-facing strings are in English.)

use crate::utils::error::AppError;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitProviderKind {
    Github,
    Gitlab,
}

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub full_path: String, // owner/name (GitHub) | namespace/path (GitLab)
    pub name: String,
    pub private: bool,
    pub default_branch: Option<String>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

impl GitProviderKind {
    pub fn parse(s: &str) -> Result<Self, AppError> {
        match s.trim().to_lowercase().as_str() {
            "github" => Ok(Self::Github),
            "gitlab" => Ok(Self::Gitlab),
            other => Err(AppError::Validation(format!("Unsupported git provider: {}", other))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
        }
    }

    pub fn default_host(&self) -> &'static str {
        match self {
            Self::Github => "github.com",
            Self::Gitlab => "gitlab.com",
        }
    }

    /// HTTPS clone URL with the token embedded, in the provider's credential format.
    pub fn https_clone_url(&self, host: &str, token: &str, repo_path: &str) -> String {
        let repo = repo_path.trim_end_matches(".git").trim_matches('/');
        match self {
            Self::Github => format!("https://x-access-token:{}@{}/{}.git", token, host, repo),
            Self::Gitlab => format!("https://oauth2:{}@{}/{}.git", token, host, repo),
        }
    }

    // --- API base + headers ---
    fn api_base(&self, host: &str) -> String {
        match self {
            Self::Github => {
                if host == "github.com" {
                    "https://api.github.com".to_string()
                } else {
                    format!("https://{}/api/v3", host) // GitHub Enterprise
                }
            }
            Self::Gitlab => format!("https://{}/api/v4", host),
        }
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder, token: &str) -> reqwest::RequestBuilder {
        let req = req.header("User-Agent", "hermes-orchestrator");
        match self {
            Self::Github => req
                .header("Authorization", format!("Bearer {}", token))
                .header("Accept", "application/vnd.github+json"),
            Self::Gitlab => req.header("PRIVATE-TOKEN", token),
        }
    }

    /// Verify a token and return the account login/username.
    pub async fn verify(&self, host: &str, token: &str, skip_tls: bool) -> Result<String, AppError> {
        let client = make_client(skip_tls)?;
        let url = format!("{}/user", self.api_base(host));
        let res = self
            .apply_auth(client.get(&url), token)
            .send()
            .await
            .map_err(|e| AppError::Validation(format!(
                "Could not reach '{}' (TLS/network error): {}. \
                 If the instance uses a self-signed certificate, enable the 'Accept insecure SSL' option.",
                host, e
            )))?;
        let status = res.status();
        if !status.is_success() {
            return Err(AppError::Validation(format!(
                "Server {} rejected the token (HTTP {}). \
                 Check the token's scopes — 'api' or 'read_api' are required.",
                host,
                status.as_u16()
            )));
        }
        #[derive(Deserialize)]
        struct GhUser { login: String }
        #[derive(Deserialize)]
        struct GlUser { username: String }
        match self {
            Self::Github => res.json::<GhUser>().await.map(|u| u.login)
                .map_err(|_| AppError::Validation("Could not read the GitHub profile.".to_string())),
            Self::Gitlab => res.json::<GlUser>().await.map(|u| u.username)
                .map_err(|_| AppError::Validation("Could not read the GitLab profile.".to_string())),
        }
    }

    pub async fn list_repos(&self, host: &str, token: &str, skip_tls: bool) -> Result<Vec<RepoInfo>, AppError> {
        let client = make_client(skip_tls)?;
        match self {
            Self::Github => {
                let url = format!("{}/user/repos?per_page=100&sort=updated", self.api_base(host));
                let res = self.apply_auth(client.get(&url), token).send().await
                    .map_err(|e| AppError::Fatal(e.into()))?;
                if !res.status().is_success() {
                    return Err(AppError::Validation("Could not list GitHub repositories.".to_string()));
                }
                #[derive(Deserialize)]
                struct GhRepo { name: String, full_name: String, private: bool, default_branch: Option<String>, html_url: Option<String> }
                let rows = res.json::<Vec<GhRepo>>().await.map_err(|e| AppError::Fatal(anyhow::anyhow!(e)))?;
                Ok(rows.into_iter().map(|r| RepoInfo {
                    full_path: r.full_name, name: r.name, private: r.private,
                    default_branch: r.default_branch, html_url: r.html_url,
                }).collect())
            }
            Self::Gitlab => {
                let url = format!("{}/projects?membership=true&per_page=100&simple=true&order_by=last_activity_at", self.api_base(host));
                let res = self.apply_auth(client.get(&url), token).send().await
                    .map_err(|e| AppError::Fatal(e.into()))?;
                if !res.status().is_success() {
                    return Err(AppError::Validation("Could not list GitLab projects.".to_string()));
                }
                #[derive(Deserialize)]
                struct GlProject { name: String, path_with_namespace: String, visibility: Option<String>, default_branch: Option<String>, web_url: Option<String> }
                let rows = res.json::<Vec<GlProject>>().await.map_err(|e| AppError::Fatal(anyhow::anyhow!(e)))?;
                Ok(rows.into_iter().map(|p| RepoInfo {
                    private: p.visibility.as_deref() != Some("public"),
                    full_path: p.path_with_namespace, name: p.name,
                    default_branch: p.default_branch, html_url: p.web_url,
                }).collect())
            }
        }
    }

    pub async fn list_branches(&self, host: &str, token: &str, repo: &str, skip_tls: bool) -> Result<Vec<String>, AppError> {
        let client = make_client(skip_tls)?;
        let url = match self {
            Self::Github => format!("{}/repos/{}/branches?per_page=100", self.api_base(host), repo),
            Self::Gitlab => format!("{}/projects/{}/repository/branches?per_page=100", self.api_base(host), enc(repo)),
        };
        let res = self.apply_auth(client.get(&url), token).send().await
            .map_err(|e| AppError::Fatal(e.into()))?;
        if !res.status().is_success() {
            return Err(AppError::Validation("Could not list branches.".to_string()));
        }
        #[derive(Deserialize)]
        struct Branch { name: String }
        let rows = res.json::<Vec<Branch>>().await.map_err(|e| AppError::Fatal(anyhow::anyhow!(e)))?;
        Ok(rows.into_iter().map(|b| b.name).collect())
    }

    /// GitLab needs an explicit ref; resolve to the project's default branch when None.
    async fn resolve_ref(&self, host: &str, token: &str, repo: &str, git_ref: Option<&str>, skip_tls: bool) -> Option<String> {
        if let Some(r) = git_ref { if !r.is_empty() { return Some(r.to_string()); } }
        match self {
            Self::Github => None, // contents API defaults to the default branch
            Self::Gitlab => {
                let client = make_client(skip_tls).ok()?;
                let url = format!("{}/projects/{}", self.api_base(host), enc(repo));
                #[derive(Deserialize)]
                struct P { default_branch: Option<String> }
                self.apply_auth(client.get(&url), token).send().await.ok()?
                    .json::<P>().await.ok()?.default_branch
            }
        }
    }

    pub async fn list_dir(&self, host: &str, token: &str, repo: &str, path: &str, git_ref: Option<&str>, skip_tls: bool) -> Result<Vec<DirEntry>, AppError> {
        let client = make_client(skip_tls)?;
        let clean = path.trim().trim_matches('/');
        match self {
            Self::Github => {
                let url = if clean.is_empty() {
                    format!("{}/repos/{}/contents", self.api_base(host), repo)
                } else {
                    format!("{}/repos/{}/contents/{}", self.api_base(host), repo, clean)
                };
                let mut req = self.apply_auth(client.get(&url), token);
                if let Some(r) = git_ref { req = req.query(&[("ref", r)]); }
                let res = req.send().await.map_err(|e| AppError::Fatal(e.into()))?;
                if !res.status().is_success() {
                    return Err(AppError::Validation("Could not read the repository contents.".to_string()));
                }
                #[derive(Deserialize)]
                struct Item { name: String, r#type: String }
                let items = res.json::<Vec<Item>>().await.map_err(|e| AppError::Fatal(anyhow::anyhow!(e)))?;
                Ok(items.into_iter().map(|i| DirEntry { name: i.name, is_dir: i.r#type == "dir" }).collect())
            }
            Self::Gitlab => {
                let r = self.resolve_ref(host, token, repo, git_ref, skip_tls).await;
                let mut q: Vec<(String, String)> = vec![("per_page".into(), "100".into())];
                if !clean.is_empty() { q.push(("path".into(), clean.to_string())); }
                if let Some(r) = r { q.push(("ref".into(), r)); }
                let url = format!("{}/projects/{}/repository/tree", self.api_base(host), enc(repo));
                let res = self.apply_auth(client.get(&url), token).query(&q).send().await
                    .map_err(|e| AppError::Fatal(e.into()))?;
                if !res.status().is_success() {
                    return Err(AppError::Validation("Could not read the GitLab project tree.".to_string()));
                }
                #[derive(Deserialize)]
                struct Item { name: String, r#type: String }
                let items = res.json::<Vec<Item>>().await.map_err(|e| AppError::Fatal(anyhow::anyhow!(e)))?;
                Ok(items.into_iter().map(|i| DirEntry { name: i.name, is_dir: i.r#type == "tree" }).collect())
            }
        }
    }

    /// Returns the file's text content, or None if it doesn't exist.
    pub async fn read_file(&self, host: &str, token: &str, repo: &str, path: &str, git_ref: Option<&str>, skip_tls: bool) -> Result<Option<String>, AppError> {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine;
        let client = make_client(skip_tls)?;
        let clean = path.trim().trim_matches('/');
        match self {
            Self::Github => {
                let url = format!("{}/repos/{}/contents/{}", self.api_base(host), repo, clean);
                let mut req = self.apply_auth(client.get(&url), token);
                if let Some(r) = git_ref { req = req.query(&[("ref", r)]); }
                let res = req.send().await.map_err(|e| AppError::Fatal(e.into()))?;
                if !res.status().is_success() { return Ok(None); }
                #[derive(Deserialize)]
                struct File { content: String, encoding: String }
                match res.json::<File>().await {
                    Ok(f) if f.encoding == "base64" => {
                        let cleaned = f.content.replace(['\n', '\r'], "");
                        Ok(BASE64.decode(cleaned).ok().and_then(|b| String::from_utf8(b).ok()))
                    }
                    _ => Ok(None),
                }
            }
            Self::Gitlab => {
                let r = self.resolve_ref(host, token, repo, git_ref, skip_tls).await.unwrap_or_else(|| "HEAD".to_string());
                let url = format!("{}/projects/{}/repository/files/{}/raw", self.api_base(host), enc(repo), enc(clean));
                let res = self.apply_auth(client.get(&url), token).query(&[("ref", r.as_str())]).send().await
                    .map_err(|e| AppError::Fatal(e.into()))?;
                if !res.status().is_success() { return Ok(None); }
                Ok(res.text().await.ok())
            }
        }
    }
}

/// Percent-encode a GitLab path segment (project id / file path).
fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

fn make_client(skip_tls: bool) -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(skip_tls)
        .build()
        .map_err(|e| AppError::Infrastructure(format!("HTTP client build failed: {}", e)))
}

/// One-time, idempotent migration of the legacy per-user `users.github_token` into
/// workspace-level encrypted credentials, backfilling existing GitHub apps. Safe to
/// run on every startup (skips workspaces that already have a GitHub credential).
pub async fn reconcile_git_credentials(pool: &sqlx::PgPool) {
    let rows = match sqlx::query!(
        "SELECT w.id AS workspace_id, w.created_by, u.github_token, u.github_username
         FROM workspaces w
         JOIN users u ON w.created_by = u.id
         WHERE u.github_token IS NOT NULL AND u.github_token <> ''
           AND NOT EXISTS (
             SELECT 1 FROM git_credentials gc
             WHERE gc.workspace_id = w.id AND gc.provider = 'github'
           )"
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return,
    };

    for r in rows {
        let token = match r.github_token {
            Some(t) if !t.trim().is_empty() => t,
            _ => continue,
        };
        let (enc_token, nonce) = match crate::utils::crypto::encrypt_env_value(token.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let cred_id = uuid::Uuid::new_v4();
        let inserted = sqlx::query!(
            "INSERT INTO git_credentials (id, workspace_id, provider, host, label, username, encrypted_token, nonce, created_by)
             VALUES ($1, $2, 'github', 'github.com', 'GitHub (migrat)', $3, $4, $5, $6)",
            cred_id, r.workspace_id, r.github_username, enc_token, nonce, r.created_by
        )
        .execute(pool)
        .await;

        if inserted.is_ok() {
            let _ = sqlx::query!(
                "UPDATE apps SET git_credential_id = $1
                 WHERE workspace_id = $2 AND git_credential_id IS NULL AND git_repository ILIKE '%github.com%'",
                cred_id, r.workspace_id
            )
            .execute(pool)
            .await;
        }
    }
}
