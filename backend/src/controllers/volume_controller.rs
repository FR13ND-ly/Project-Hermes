use axum::{
    extract::{State, Path},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::models::volume_model::AppVolume;
use crate::dtos::volume_dto::{CreateVolumeRequest, VolumeResponse};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

const VOLUMES_BASE_DIR: &str = "/var/lib/hermes/volumes";

pub async fn create_volume(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateVolumeRequest>,
) -> Result<(StatusCode, Json<VolumeResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!(
        "SELECT id FROM apps WHERE id = $1 AND workspace_id = $2",
        payload.app_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    let volume_id = Uuid::new_v4();
    let host_path = format!("{}/{}", VOLUMES_BASE_DIR, volume_id);

    std::fs::create_dir_all(&host_path).map_err(|e| {
        AppError::Fatal(anyhow::anyhow!("Failed to physically create volume directory: {}", e))
    })?;

    sqlx::query!(
        "INSERT INTO app_volumes (id, workspace_id, app_id, name, container_path, host_path)
         VALUES ($1, $2, $3, $4, $5, $6)",
        volume_id, ws_id, payload.app_id, payload.name.trim(), payload.container_path.trim(), host_path
    )
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(VolumeResponse {
            id: volume_id,
            app_id: payload.app_id,
            name: payload.name,
            container_path: payload.container_path,
            host_path,
        }),
    ))
}

pub async fn list_app_volumes(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
) -> Result<Json<Vec<VolumeResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let volumes = sqlx::query_as::<_, AppVolume>(
        "SELECT id, workspace_id, app_id, name, container_path, host_path, created_at 
         FROM app_volumes 
         WHERE app_id = $1 AND workspace_id = $2 
         ORDER BY created_at DESC"
    )
    .bind(app_id)
    .bind(ws_id)
    .fetch_all(&state.pool)
    .await?;

    let response = volumes
        .into_iter()
        .map(|v| VolumeResponse {
            id: v.id,
            app_id: v.app_id,
            name: v.name,
            container_path: v.container_path,
            host_path: v.host_path,
        })
        .collect();

    Ok(Json(response))
}

pub async fn delete_volume(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(volume_id): Path<Uuid>,
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

    sqlx::query!("DELETE FROM app_volumes WHERE id = $1", volume_id)
        .execute(&state.pool)
        .await?;

    let _ = std::fs::remove_dir_all(&volume.host_path);

    Ok(StatusCode::NO_CONTENT)
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
                    return Err(AppError::Validation("Acces neautorizat în afara directorului de bază.".to_string()));
                }
            }
            _ => {}
        }
    }
    if resolved.starts_with(base) {
        Ok(resolved)
    } else {
        Err(AppError::Validation("Acces neautorizat în afara directorului de bază.".to_string()))
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
    let mut dir = std::fs::read_dir(target_path).map_err(|e| AppError::Fatal(anyhow::anyhow!("Eroare la citirea directorului: {}", e)))?;
    while let Some(entry) = dir.next() {
        if let Ok(entry) = entry {
            let metadata = entry.metadata().map_err(|e| AppError::Fatal(anyhow::anyhow!("Eroare la citirea metadatelor: {}", e)))?;
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

use axum::extract::Multipart;

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

    std::fs::create_dir_all(&target_dir).map_err(|e| AppError::Fatal(anyhow::anyhow!("Eroare la crearea directorului destinație: {}", e)))?;

    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::Fatal(anyhow::anyhow!("Multipart reading error: {}", e)))? {
        let name = field.file_name().unwrap_or("unnamed_file").to_string();
        let clean_name = std::path::Path::new(&name)
            .file_name()
            .ok_or_else(|| AppError::Validation("Nume de fișier invalid.".to_string()))?
            .to_string_lossy()
            .to_string();

        let filepath = target_dir.join(&clean_name);
        
        if !filepath.starts_with(base_path) {
            return Err(AppError::Validation("Path transversal blocked.".to_string()));
        }

        let data = field.bytes().await.map_err(|e| AppError::Fatal(anyhow::anyhow!("Multipart bytes reading error: {}", e)))?;
        std::fs::write(&filepath, data).map_err(|e| AppError::Fatal(anyhow::anyhow!("Eroare la salvarea fișierului pe host: {}", e)))?;
    }

    Ok(StatusCode::OK)
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
        return Err(AppError::NotFound("Fișierul specificat nu există.".to_string()));
    }

    let filename = target_file.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());

    let file = tokio::fs::File::open(&target_file).await
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Nu s-a putut deschide fișierul: {}", e)))?;
    
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/octet-stream"),
    );
    let cd_val = axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename))
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Eroare la procesarea header-ului: {}", e)))?;
    headers.insert(axum::http::header::CONTENT_DISPOSITION, cd_val);

    Ok((headers, body))
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
        return Err(AppError::Validation("Nu puteți șterge directorul rădăcină al volumului.".to_string()));
    }

    let target_path = safe_resolve(base_path, &query.path)?;

    if !target_path.exists() {
        return Err(AppError::NotFound("Fișierul sau directorul nu există.".to_string()));
    }

    if target_path.is_dir() {
        std::fs::remove_dir_all(&target_path).map_err(|e| {
            AppError::Fatal(anyhow::anyhow!("Eroare la ștergerea recursivă a folderului: {}", e))
        })?;
    } else {
        std::fs::remove_file(&target_path).map_err(|e| {
            AppError::Fatal(anyhow::anyhow!("Eroare la ștergerea fișierului: {}", e))
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
        .ok_or_else(|| AppError::Validation("Nume director invalid.".to_string()))?
        .to_string_lossy()
        .to_string();

    let new_dir_path = target_dir.join(clean_name);
    
    if !new_dir_path.starts_with(base_path) {
        return Err(AppError::Validation("Path transversal blocked.".to_string()));
    }

    std::fs::create_dir_all(&new_dir_path).map_err(|e| {
        AppError::Fatal(anyhow::anyhow!("Eroare la crearea directorului: {}", e))
    })?;

    Ok(StatusCode::CREATED)
}