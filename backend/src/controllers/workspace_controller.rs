use axum::{extract::{State, Path}, http::StatusCode, Json};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dtos::workspace_dto::{CreateWorkspaceRequest, WorkspaceResponse, WorkspaceUsageResponse, UpdateWorkspaceRequest};
use crate::dtos::workspace_member_dto::{AddMemberRequest, UpdateMemberRoleRequest, WorkspaceMemberResponse};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::models::workspace_model::Workspace;
use crate::utils::error::AppError;
use crate::config::platform_defaults::{WORKSPACE_MAX_MEMORY_MB, WORKSPACE_MAX_STORAGE_GB};

pub async fn create_workspace(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceResponse>), AppError> {
    if !claims.is_super_admin {
        return Err(AppError::Permission("Only platform administrators can create workspaces.".to_string()));
    }

    let slug = payload.name.to_lowercase().trim().replace(" ", "-");
    let workspace_id = Uuid::new_v4();

    // Rezolvată eroarea de tip: adăugat unwrap_or(false) pentru Option<bool>
    let slug_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM workspaces WHERE slug = $1)",
        slug
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if slug_exists {
        return Err(AppError::Conflict("A workspace with a similar name already exists".to_string()));
    }

    let mut tx = state.pool.begin().await?;

    sqlx::query!(
        "INSERT INTO workspaces (id, name, slug, created_by, max_memory_mb, max_storage_gb) VALUES ($1, $2, $3, $4, $5, $6)",
        workspace_id, payload.name, slug, claims.sub, WORKSPACE_MAX_MEMORY_MB, WORKSPACE_MAX_STORAGE_GB
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "INSERT INTO workspace_members (workspace_id, user_id, role_id) VALUES ($1, $2, (SELECT id FROM roles WHERE name = 'owner'))",
        workspace_id, claims.sub
    )
    .execute(&mut *tx)
    .await?;

    if claims.current_workspace_id.is_none() {
        sqlx::query!(
            "UPDATE users SET current_workspace_id = $1 WHERE id = $2",
            workspace_id, claims.sub
        )
        .execute(&mut *tx)
        .await?;
    }

    let client = crate::utils::k8s::K8sManager::get_client().await?;
    let k8s_ns_name = format!("hermes-ws-{}", workspace_id);
    crate::utils::k8s::K8sManager::create_namespace(&client, &k8s_ns_name, WORKSPACE_MAX_MEMORY_MB, WORKSPACE_MAX_STORAGE_GB).await?;

    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(WorkspaceResponse {
            id: workspace_id,
            name: payload.name,
            slug,
            max_memory_mb: WORKSPACE_MAX_MEMORY_MB,
            max_storage_gb: WORKSPACE_MAX_STORAGE_GB,
            cloudflare_api_token: None,
            cloudflare_zone_id: None,
            ingress_ip: None,
            base_domain: None,
        }),
    ))
}

pub async fn list_my_workspaces(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<Vec<WorkspaceResponse>>, AppError> {
    // Corectată eroarea de argumente: parametrii se trimit prin .bind() în SQLx 0.7+
    let workspaces = sqlx::query_as::<_, Workspace>(
        "SELECT w.* FROM workspaces w JOIN workspace_members wm ON w.id = wm.workspace_id WHERE wm.user_id = $1"
    )
    .bind(claims.sub)
    .fetch_all(&state.pool)
    .await?;

    let response = workspaces
        .into_iter()
        .map(|w| WorkspaceResponse {
            id: w.id,
            name: w.name,
            slug: w.slug,
            max_memory_mb: w.max_memory_mb,
            max_storage_gb: w.max_storage_gb,
            cloudflare_api_token: w.cloudflare_api_token,
            cloudflare_zone_id: w.cloudflare_zone_id,
            ingress_ip: w.ingress_ip,
            base_domain: w.base_domain,
        })
        .collect();

    Ok(Json(response))
}

pub async fn get_workspace_usage(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<WorkspaceUsageResponse>, AppError> {
    let ws_id = match claims.current_workspace_id {
        Some(id) => id,
        None => {
            return Ok(Json(WorkspaceUsageResponse {
                workspace_id: Uuid::nil(),
                max_memory_mb: 0,
                used_memory_mb: 0,
                max_storage_gb: 0,
                used_storage_gb: 0,
            }));
        }
    };

    let limits = sqlx::query!(
        "SELECT max_memory_mb, max_storage_gb FROM workspaces WHERE id = $1",
        ws_id
    )
    .fetch_one(&state.pool)
    .await?;

    // Query real K8s namespace usage
    let namespace = format!("hermes-ws-{}", ws_id);
    let (mut used_memory, mut used_storage_gb) = if let Ok(client) = crate::utils::k8s::K8sManager::get_client().await {
        let (mem_mb, _) = crate::utils::k8s::K8sManager::get_namespace_resource_usage(&client, &namespace).await;
        let storage_gb = crate::utils::k8s::K8sManager::get_namespace_storage_usage_gb(&client, &namespace).await;
        (mem_mb, storage_gb)
    } else {
        (0, 0.0)
    };

    // Fallback to database allocation if K8s reports 0 (common in dev/local environment)
    if used_memory == 0 || used_storage_gb == 0.0 {
        let db_usage = sqlx::query!(
            "SELECT 
                (SELECT COALESCE(SUM(ai.memory_limit_mb), 0)::bigint 
                 FROM app_instances ai 
                 JOIN apps a ON ai.app_id = a.id 
                 WHERE a.workspace_id = $1 AND ai.status = 'running') as \"app_mem!\",
                (SELECT COALESCE(SUM(d.memory_limit_mb), 0)::bigint 
                 FROM databases d 
                 WHERE d.workspace_id = $1 AND d.status = 'running') as \"db_mem!\",
                (
                    (SELECT COALESCE(COUNT(*), 0) 
                     FROM databases d 
                     WHERE d.workspace_id = $1 AND d.status = 'running')
                    +
                    (SELECT COALESCE(COUNT(*), 0) 
                     FROM app_volumes av
                     JOIN app_instances ai ON av.app_id = ai.app_id
                     WHERE av.workspace_id = $1 AND ai.status = 'running')
                ) as \"db_storage!\"",
            ws_id
        )
        .fetch_one(&state.pool)
        .await?;

        if used_memory == 0 {
            used_memory = (db_usage.app_mem + db_usage.db_mem) as i32;
        }
        if used_storage_gb == 0.0 {
            used_storage_gb = db_usage.db_storage as f64;
        }
    }

    // Round storage to 2 decimal places for display
    let used_storage_rounded = (used_storage_gb * 100.0).round() / 100.0;

    Ok(Json(WorkspaceUsageResponse {
        workspace_id: ws_id,
        max_memory_mb: limits.max_memory_mb,
        used_memory_mb: used_memory,
        max_storage_gb: limits.max_storage_gb,
        used_storage_gb: used_storage_rounded as i32,
    }))
}


pub async fn get_current_workspace(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<WorkspaceResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let w = sqlx::query_as::<_, Workspace>(
        "SELECT * FROM workspaces WHERE id = $1"
    )
    .bind(ws_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(WorkspaceResponse {
        id: w.id,
        name: w.name,
        slug: w.slug,
        max_memory_mb: w.max_memory_mb,
        max_storage_gb: w.max_storage_gb,
        cloudflare_api_token: w.cloudflare_api_token,
        cloudflare_zone_id: w.cloudflare_zone_id,
        ingress_ip: w.ingress_ip,
        base_domain: w.base_domain,
    }))
}

pub async fn update_workspace_settings(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Load current workspace
    let mut current = sqlx::query_as::<_, Workspace>(
        "SELECT * FROM workspaces WHERE id = $1"
    )
    .bind(ws_id)
    .fetch_one(&state.pool)
    .await?;

    // Apply updates if present
    if let Some(ref name) = payload.name {
        current.name = name.clone();
        current.slug = name.to_lowercase().trim().replace(" ", "-");
    }
    if let Some(max_mem) = payload.max_memory_mb {
        current.max_memory_mb = max_mem;
    }
    if let Some(max_store) = payload.max_storage_gb {
        current.max_storage_gb = max_store;
    }
    current.cloudflare_api_token = payload.cloudflare_api_token;
    current.cloudflare_zone_id = payload.cloudflare_zone_id;
    current.ingress_ip = payload.ingress_ip;
    current.base_domain = payload.base_domain;
 
    sqlx::query!(
        "UPDATE workspaces 
         SET name = $1, slug = $2, max_memory_mb = $3, max_storage_gb = $4, 
             cloudflare_api_token = $5, cloudflare_zone_id = $6, ingress_ip = $7, base_domain = $8, updated_at = now()
         WHERE id = $9",
        current.name,
        current.slug,
        current.max_memory_mb,
        current.max_storage_gb,
        current.cloudflare_api_token,
        current.cloudflare_zone_id,
        current.ingress_ip,
        current.base_domain,
        ws_id
    )
    .execute(&state.pool)
    .await?;
 
    // Propagate limits to Kubernetes namespace immediately
    if let Ok(client) = crate::utils::k8s::K8sManager::get_client().await {
        let k8s_ns_name = format!("hermes-ws-{}", ws_id);
        let _ = crate::utils::k8s::K8sManager::apply_namespace_limits(
            &client,
            &k8s_ns_name,
            current.max_memory_mb,
            current.max_storage_gb,
        )
        .await;
    }
 
    Ok(Json(WorkspaceResponse {
        id: current.id,
        name: current.name,
        slug: current.slug,
        max_memory_mb: current.max_memory_mb,
        max_storage_gb: current.max_storage_gb,
        cloudflare_api_token: current.cloudflare_api_token,
        cloudflare_zone_id: current.cloudflare_zone_id,
        ingress_ip: current.ingress_ip,
        base_domain: current.base_domain,
    }))
}

pub async fn list_workspace_members(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<Vec<WorkspaceMemberResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let members = sqlx::query!(
        "SELECT u.id as user_id, u.email, u.username, r.name as role_name
         FROM workspace_members wm
         JOIN users u ON wm.user_id = u.id
         JOIN roles r ON wm.role_id = r.id
         WHERE wm.workspace_id = $1
         ORDER BY u.email ASC",
        ws_id
    )
    .fetch_all(&state.pool)
    .await?;

    let response = members
        .into_iter()
        .map(|m| WorkspaceMemberResponse {
            user_id: m.user_id,
            email: m.email,
            username: m.username,
            role_name: m.role_name,
        })
        .collect();

    Ok(Json(response))
}

pub async fn add_workspace_member(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<AddMemberRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let user_email = payload.email.trim().to_lowercase();
    let target_user = sqlx::query!(
        "SELECT id FROM users WHERE email = $1",
        user_email
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("User with this email not found.".to_string()))?;

    let already_member = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM workspace_members WHERE workspace_id = $1 AND user_id = $2)",
        ws_id, target_user.id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if already_member {
        return Err(AppError::Conflict("User is already a member of this workspace.".to_string()));
    }

    let role_name = payload.role_name.trim().to_lowercase();
    let role = sqlx::query!(
        "SELECT id FROM roles WHERE name = $1",
        role_name
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Validation("Invalid role name specified.".to_string()))?;

    sqlx::query!(
        "INSERT INTO workspace_members (workspace_id, user_id, role_id) VALUES ($1, $2, $3)",
        ws_id, target_user.id, role.id
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::CREATED)
}

pub async fn update_workspace_member_role(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(user_id): Path<Uuid>,
    Json(payload): Json<UpdateMemberRoleRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let owner = sqlx::query_scalar!(
        "SELECT created_by FROM workspaces WHERE id = $1",
        ws_id
    )
    .fetch_one(&state.pool)
    .await?;

    if user_id == owner {
        return Err(AppError::Validation("Cannot modify the role of the workspace owner.".to_string()));
    }

    let member_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM workspace_members WHERE workspace_id = $1 AND user_id = $2)",
        ws_id, user_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !member_exists {
        return Err(AppError::NotFound("Member not found in this workspace.".to_string()));
    }

    let role_name = payload.role_name.trim().to_lowercase();
    let role = sqlx::query!(
        "SELECT id FROM roles WHERE name = $1",
        role_name
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Validation("Invalid role name specified.".to_string()))?;

    sqlx::query!(
        "UPDATE workspace_members SET role_id = $1 WHERE workspace_id = $2 AND user_id = $3",
        role.id, ws_id, user_id
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::OK)
}

pub async fn remove_workspace_member(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(user_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let owner = sqlx::query_scalar!(
        "SELECT created_by FROM workspaces WHERE id = $1",
        ws_id
    )
    .fetch_one(&state.pool)
    .await?;

    if user_id == owner {
        return Err(AppError::Validation("Cannot remove the workspace owner.".to_string()));
    }

    if user_id == claims.sub {
        return Err(AppError::Validation("Cannot remove yourself from the workspace.".to_string()));
    }

    let rows_affected = sqlx::query!(
        "DELETE FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
        ws_id, user_id
    )
    .execute(&state.pool)
    .await?
    .rows_affected();

    if rows_affected == 0 {
        return Err(AppError::NotFound("Member not found in this workspace.".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_workspace(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let is_super_admin = claims.is_super_admin;
    
    let is_owner = if !is_super_admin {
        let owner_role_id = sqlx::query_scalar!(
            "SELECT id FROM roles WHERE name = 'owner'"
        )
        .fetch_one(&state.pool)
        .await?;
        
        sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM workspace_members WHERE workspace_id = $1 AND user_id = $2 AND role_id = $3)",
            workspace_id, claims.sub, owner_role_id
        )
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(false)
    } else {
        true
    };

    if !is_owner {
        return Err(AppError::Permission("Only workspace owners or super admins can delete workspaces.".to_string()));
    }

    // 1. Delete Kubernetes Namespace
    let client = crate::utils::k8s::K8sManager::get_client().await?;
    let k8s_ns_name = format!("hermes-ws-{}", workspace_id);
    let _ = crate::utils::k8s::K8sManager::delete_namespace(&client, &k8s_ns_name).await;

    // 2. Clean up physical storage buckets directories on host
    if let Ok(buckets) = sqlx::query!(
        "SELECT slug, access_type::text as \"access_type!\" FROM storage_buckets WHERE workspace_id = $1",
        workspace_id
    )
    .fetch_all(&state.pool)
    .await {
        for bucket in buckets {
            let bucket_access = match bucket.access_type.as_str() {
                "public_assets" => crate::models::storage_model::BucketAccessType::PublicAssets,
                "private_storage" => crate::models::storage_model::BucketAccessType::PrivateStorage,
                "static_website" => crate::models::storage_model::BucketAccessType::StaticWebsite,
                _ => crate::models::storage_model::BucketAccessType::AppBounded,
            };
            let _ = crate::utils::storage_engine::StorageEngine::delete_bucket_physical(&workspace_id.to_string(), &bucket.slug, &bucket_access).await;
        }
    }

    // 3. Clean up physical database backups on host disk
    let backups_dir = format!("/var/lib/hermes/backups");
    if let Ok(dbs) = sqlx::query!("SELECT id FROM databases WHERE workspace_id = $1", workspace_id)
        .fetch_all(&state.pool)
        .await {
        for db in dbs {
            let db_backup_path = format!("{}/{}", backups_dir, db.id);
            let _ = std::fs::remove_dir_all(db_backup_path);
        }
    }

    // 4. Cascade delete tables in Postgres (using transaction)
    let mut tx = state.pool.begin().await?;

    // Delete environment variables
    sqlx::query!("DELETE FROM environment_variables WHERE workspace_id = $1", workspace_id)
        .execute(&mut *tx)
        .await?;

    // Delete domains
    sqlx::query!("DELETE FROM domains WHERE workspace_id = $1", workspace_id)
        .execute(&mut *tx)
        .await?;

    // Delete storage objects and buckets
    sqlx::query!(
        "DELETE FROM storage_objects WHERE bucket_id IN (SELECT id FROM storage_buckets WHERE workspace_id = $1)",
        workspace_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM storage_buckets WHERE workspace_id = $1", workspace_id)
        .execute(&mut *tx)
        .await?;

    // Delete database backups and databases
    sqlx::query!(
        "DELETE FROM database_backups WHERE database_id IN (SELECT id FROM databases WHERE workspace_id = $1)",
        workspace_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM databases WHERE workspace_id = $1", workspace_id)
        .execute(&mut *tx)
        .await?;

    // Delete app builds, app instances and apps
    sqlx::query!(
        "DELETE FROM app_builds WHERE app_id IN (SELECT id FROM apps WHERE workspace_id = $1)",
        workspace_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM app_instances WHERE app_id IN (SELECT id FROM apps WHERE workspace_id = $1)",
        workspace_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM app_volumes WHERE workspace_id = $1",
        workspace_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM apps WHERE workspace_id = $1", workspace_id)
        .execute(&mut *tx)
        .await?;

    // Delete projects
    sqlx::query!("DELETE FROM projects WHERE workspace_id = $1", workspace_id)
        .execute(&mut *tx)
        .await?;

    // Delete members
    sqlx::query!("DELETE FROM workspace_members WHERE workspace_id = $1", workspace_id)
        .execute(&mut *tx)
        .await?;

    // Reset current_workspace_id for any users who had this workspace selected
    sqlx::query!(
        "UPDATE users SET current_workspace_id = NULL WHERE current_workspace_id = $1",
        workspace_id
    )
    .execute(&mut *tx)
    .await?;

    // Set some other workspace for these users if they have any, otherwise leave as NULL
    sqlx::query!(
        "UPDATE users u 
         SET current_workspace_id = (SELECT workspace_id FROM workspace_members WHERE user_id = u.id LIMIT 1)
         WHERE u.current_workspace_id IS NULL"
    )
    .execute(&mut *tx)
    .await?;

    // Finally delete workspace
    sqlx::query!("DELETE FROM workspaces WHERE id = $1", workspace_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminWorkspaceStatsResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub max_memory_mb: i32,
    pub max_storage_gb: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub creator: Option<String>,
    pub member_count: i64,
    pub app_count: i64,
    pub active_app_count: i64,
    pub database_count: i64,
    pub allocated_memory_mb: i64,
    pub allocated_storage_gb: i64,
}

pub async fn admin_list_all_workspaces(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<Vec<AdminWorkspaceStatsResponse>>, AppError> {
    if !claims.is_super_admin {
        return Err(AppError::Permission("Access denied. Admin privileges required.".to_string()));
    }

    let records = sqlx::query!(
        "SELECT 
            w.id, 
            w.name, 
            w.slug, 
            w.max_memory_mb, 
            w.max_storage_gb, 
            w.created_at, 
            (SELECT username FROM users WHERE id = w.created_by) as creator,
            (SELECT COUNT(*) FROM workspace_members WHERE workspace_id = w.id) as \"member_count!\",
            (SELECT COUNT(*) FROM apps WHERE workspace_id = w.id) as \"app_count!\",
            (SELECT COUNT(*) FROM app_instances ai JOIN apps a ON ai.app_id = a.id WHERE a.workspace_id = w.id AND ai.status = 'running') as \"active_app_count!\",
            (SELECT COUNT(*) FROM databases WHERE workspace_id = w.id) as \"database_count!\",
            COALESCE((SELECT SUM(ai.memory_limit_mb) FROM app_instances ai JOIN apps a ON ai.app_id = a.id WHERE a.workspace_id = w.id), 0)::bigint as \"app_mem!\",
            COALESCE((SELECT SUM(d.memory_limit_mb) FROM databases d WHERE d.workspace_id = w.id), 0)::bigint as \"db_mem!\",
            (
                (SELECT COALESCE(COUNT(*), 0) FROM databases d WHERE d.workspace_id = w.id)
                +
                (SELECT COALESCE(COUNT(*), 0) FROM app_volumes av WHERE av.workspace_id = w.id)
            )::bigint as \"allocated_storage!\"
         FROM workspaces w
         ORDER BY w.name ASC"
    )
    .fetch_all(&state.pool)
    .await?;

    let response = records
        .into_iter()
        .map(|r| AdminWorkspaceStatsResponse {
            id: r.id,
            name: r.name,
            slug: r.slug,
            max_memory_mb: r.max_memory_mb,
            max_storage_gb: r.max_storage_gb,
            created_at: r.created_at,
            creator: r.creator,
            member_count: r.member_count,
            app_count: r.app_count,
            active_app_count: r.active_app_count,
            database_count: r.database_count,
            allocated_memory_mb: r.app_mem + r.db_mem,
            allocated_storage_gb: r.allocated_storage,
        })
        .collect();

    Ok(Json(response))
}