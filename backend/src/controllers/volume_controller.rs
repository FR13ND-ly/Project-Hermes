use axum::{
    extract::{State, Path, Multipart},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dtos::volume_dto::ProjectVolumeResponse;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

/// All persistent volumes (PVCs) across a project's apps. PVCs are created only
/// automatically at build (from Dockerfile VOLUME directives); this endpoint
/// powers the central Storage interface that lists and browses them.
pub async fn list_project_volumes(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<ProjectVolumeResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let rows = sqlx::query!(
        "SELECT v.id, v.app_id, a.name AS app_name, v.name, v.container_path, v.host_path
         FROM app_volumes v
         JOIN apps a ON v.app_id = a.id
         WHERE a.project_id = $1 AND a.workspace_id = $2
         ORDER BY a.name ASC, v.name ASC",
        project_id,
        ws_id
    )
    .fetch_all(&state.pool)
    .await?;

    let list = rows
        .into_iter()
        .map(|r| {
            let is_auto = r.name.starts_with("auto-");
            ProjectVolumeResponse {
                id: r.id,
                app_id: r.app_id,
                app_name: r.app_name,
                name: r.name,
                container_path: r.container_path,
                host_path: r.host_path,
                is_auto,
            }
        })
        .collect();

    Ok(Json(list))
}

fn safe_resolve(base: &std::path::Path, relative: &str) -> Result<std::path::PathBuf, AppError> {
    let mut resolved = base.to_path_buf();
    for component in std::path::Path::new(relative).components() {
        match component {
            std::path::Component::Normal(p) => {
                resolved.push(p);
            }
            std::path::Component::ParentDir => {
                if resolved.starts_with(base) && resolved != base {
                    resolved.pop();
                } else {
                    return Err(AppError::Validation("Unauthorized access outside the base directory.".to_string()));
                }
            }
            _ => {}
        }
    }
    if resolved.starts_with(base) {
        Ok(resolved)
    } else {
        Err(AppError::Validation("Unauthorized access outside the base directory.".to_string()))
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct ListFilesQuery {
    pub path: Option<String>,
}

pub async fn list_volume_files(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(volume_id): Path<Uuid>,
    axum::extract::Query(query): axum::extract::Query<ListFilesQuery>,
) -> Result<Json<Vec<crate::dtos::volume_explorer_dto::FileItem>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let volume = sqlx::query!(
        "SELECT host_path FROM app_volumes WHERE id = $1 AND workspace_id = $2",
        volume_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Persistent volume not found.".to_string()))?;

    let base_path = std::path::Path::new(&volume.host_path);
    let rel_path = query.path.unwrap_or_default();
    let target_path = safe_resolve(base_path, &rel_path)?;

    if !target_path.exists() {
        return Ok(Json(vec![]));
    }

    let mut items = Vec::new();
    let mut dir = std::fs::read_dir(target_path).map_err(|e| AppError::Fatal(anyhow::anyhow!("Error reading the directory: {}", e)))?;
    while let Some(entry) = dir.next() {
        if let Ok(entry) = entry {
            let metadata = entry.metadata().map_err(|e| AppError::Fatal(anyhow::anyhow!("Error reading metadata: {}", e)))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = metadata.is_dir();
            let size_bytes = if is_dir { 0 } else { metadata.len() };
            let modified_time = metadata.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            items.push(crate::dtos::volume_explorer_dto::FileItem {
                name,
                is_dir,
                size_bytes,
                modified_time,
            });
        }
    }

    items.sort_by(|a, b| {
        if a.is_dir && !b.is_dir {
            std::cmp::Ordering::Less
        } else if !a.is_dir && b.is_dir {
            std::cmp::Ordering::Greater
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });

    Ok(Json(items))
}

use axum::response::IntoResponse;
use axum::body::Body;

#[derive(Debug, serde::Deserialize)]
pub struct DownloadFileQuery {
    pub path: String,
}

pub async fn download_volume_file(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(volume_id): Path<Uuid>,
    axum::extract::Query(query): axum::extract::Query<DownloadFileQuery>,
) -> Result<impl IntoResponse, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let volume = sqlx::query!(
        "SELECT host_path FROM app_volumes WHERE id = $1 AND workspace_id = $2",
        volume_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Persistent volume not found.".to_string()))?;

    let base_path = std::path::Path::new(&volume.host_path);
    let target_file = safe_resolve(base_path, &query.path)?;

    if !target_file.exists() || target_file.is_dir() {
        return Err(AppError::NotFound("The specified file does not exist.".to_string()));
    }

    let filename = target_file.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());

    let file = tokio::fs::File::open(&target_file).await
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Could not open the file: {}", e)))?;
    
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/octet-stream"),
    );
    let cd_val = axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename))
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Error processing the header: {}", e)))?;
    headers.insert(axum::http::header::CONTENT_DISPOSITION, cd_val);

    Ok((headers, body))
}

#[derive(Debug, serde::Deserialize)]
pub struct UploadFileQuery {
    pub path: Option<String>,
}

pub async fn upload_volume_file(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(volume_id): Path<Uuid>,
    axum::extract::Query(query): axum::extract::Query<UploadFileQuery>,
    mut multipart: Multipart,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let volume = sqlx::query!(
        "SELECT host_path FROM app_volumes WHERE id = $1 AND workspace_id = $2",
        volume_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Persistent volume not found.".to_string()))?;

    let base_path = std::path::Path::new(&volume.host_path);
    let rel_path = query.path.unwrap_or_default();
    let target_dir = safe_resolve(base_path, &rel_path)?;

    std::fs::create_dir_all(&target_dir).map_err(|e| AppError::Fatal(anyhow::anyhow!("Error creating the destination directory: {}", e)))?;

    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::Fatal(anyhow::anyhow!("Multipart reading error: {}", e)))? {
        let name = field.file_name().unwrap_or("unnamed_file").to_string();
        let clean_name = std::path::Path::new(&name)
            .file_name()
            .ok_or_else(|| AppError::Validation("Invalid file name.".to_string()))?
            .to_string_lossy()
            .to_string();

        let filepath = target_dir.join(&clean_name);

        if !filepath.starts_with(base_path) {
            return Err(AppError::Validation("Path transversal blocked.".to_string()));
        }

        let data = field.bytes().await.map_err(|e| AppError::Fatal(anyhow::anyhow!("Multipart bytes reading error: {}", e)))?;
        std::fs::write(&filepath, data).map_err(|e| AppError::Fatal(anyhow::anyhow!("Error saving the file on the host: {}", e)))?;
    }

    Ok(StatusCode::OK)
}

#[derive(Debug, serde::Deserialize)]
pub struct DeleteFileQuery {
    pub path: String,
}

pub async fn delete_volume_file(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(volume_id): Path<Uuid>,
    axum::extract::Query(query): axum::extract::Query<DeleteFileQuery>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let volume = sqlx::query!(
        "SELECT host_path FROM app_volumes WHERE id = $1 AND workspace_id = $2",
        volume_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Persistent volume not found.".to_string()))?;

    let base_path = std::path::Path::new(&volume.host_path);

    if query.path.trim().is_empty() || query.path == "/" || query.path == "." {
        return Err(AppError::Validation("You cannot delete the volume's root directory.".to_string()));
    }

    let target_path = safe_resolve(base_path, &query.path)?;

    if !target_path.exists() {
        return Err(AppError::NotFound("The file or directory does not exist.".to_string()));
    }

    if target_path.is_dir() {
        std::fs::remove_dir_all(&target_path).map_err(|e| {
            AppError::Fatal(anyhow::anyhow!("Error recursively deleting the folder: {}", e))
        })?;
    } else {
        std::fs::remove_file(&target_path).map_err(|e| {
            AppError::Fatal(anyhow::anyhow!("Error deleting the file: {}", e))
        })?;
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDirectoryRequest {
    pub path: Option<String>,
    pub name: String,
}

pub async fn create_volume_directory(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(volume_id): Path<Uuid>,
    Json(payload): Json<CreateDirectoryRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let volume = sqlx::query!(
        "SELECT host_path FROM app_volumes WHERE id = $1 AND workspace_id = $2",
        volume_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Persistent volume not found.".to_string()))?;

    let base_path = std::path::Path::new(&volume.host_path);
    let rel_path = payload.path.unwrap_or_default();
    let target_dir = safe_resolve(base_path, &rel_path)?;

    let clean_name = std::path::Path::new(&payload.name)
        .file_name()
        .ok_or_else(|| AppError::Validation("Invalid directory name.".to_string()))?
        .to_string_lossy()
        .to_string();

    let new_dir_path = target_dir.join(clean_name);

    if !new_dir_path.starts_with(base_path) {
        return Err(AppError::Validation("Path transversal blocked.".to_string()));
    }

    std::fs::create_dir_all(&new_dir_path).map_err(|e| {
        AppError::Fatal(anyhow::anyhow!("Error creating the directory: {}", e))
    })?;

    Ok(StatusCode::CREATED)
}