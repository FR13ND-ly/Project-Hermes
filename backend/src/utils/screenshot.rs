//! Vercel-style preview screenshots of deployed app instances.
//!
//! When a deploy becomes healthy we render the app's *in-cluster* Service URL with a
//! one-shot headless-Chromium Job and drop the PNG on the shared `hermes-backups` PVC —
//! the same co-mounted volume the DB backups use, so the control plane reads the file
//! back from its own mount with no pod exec (which fails on this cluster). Because we
//! hit the in-cluster Service directly, this works for every running app, with or
//! without a public domain.

use std::path::Path;
use uuid::Uuid;

use crate::utils::error::AppError;
use crate::utils::k8s::K8sManager;

/// Where screenshots live on the shared backups PVC (same mount in Job + control plane).
const SCREENSHOTS_SUBDIR: &str = "screenshots";
const PVC_MOUNT: &str = "/var/lib/hermes/backups";
/// Headless-Chromium image; small Alpine build, ships `chromium-browser` on PATH.
const CHROME_IMAGE: &str = "zenika/alpine-chrome:latest";

/// Absolute path the control plane reads (and the Job writes) for a given instance.
pub fn screenshot_path_for(instance_id: Uuid) -> String {
    format!("{}/{}/{}.png", PVC_MOUNT, SCREENSHOTS_SUBDIR, instance_id)
}

/// Fire-and-forget capture: spawned off the deploy path so a screenshot problem never
/// affects the deploy outcome. Every failure is logged and swallowed.
pub async fn capture_instance_screenshot(
    pool: sqlx::PgPool,
    app_name: String,
    namespace: String,
    instance_id: Uuid,
) {
    match try_capture(&pool, &app_name, &namespace, instance_id).await {
        Ok(()) => tracing::info!(%instance_id, "Captured preview screenshot"),
        Err(e) => tracing::warn!(%instance_id, "Preview screenshot capture failed: {}", e),
    }
}

async fn try_capture(
    pool: &sqlx::PgPool,
    app_name: &str,
    namespace: &str,
    instance_id: Uuid,
) -> Result<(), AppError> {
    // The in-cluster Service shares the deployment name (`app=<name>` selector), so the
    // app is reachable at <name>.<namespace>.svc.cluster.local:<internal_port>.
    let port: i32 = sqlx::query_scalar("SELECT internal_port FROM app_instances WHERE id = $1")
        .bind(instance_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Instance no longer exists.".to_string()))?;

    let url = format!("http://{}.{}.svc.cluster.local:{}", app_name, namespace, port);
    let out_path = screenshot_path_for(instance_id);

    let client = K8sManager::get_client().await?;
    let system_ns = K8sManager::system_namespace();
    let job_name = format!(
        "shot-{}-{}",
        &instance_id.to_string()[..8],
        chrono::Utc::now().format("%H%M%S")
    );

    // Wait a moment for the app to settle, then render at a 16:10 desktop size. The
    // single-quoted URL/paths are safe: both are server-controlled (UUID + Service DNS).
    let command = format!(
        "mkdir -p {dir}/{sub} && \
         chromium-browser --headless --no-sandbox --disable-gpu --disable-dev-shm-usage \
         --hide-scrollbars --force-color-profile=srgb --window-size=1280,800 \
         --virtual-time-budget=10000 --timeout=30000 --screenshot='{out}' '{url}'",
        dir = PVC_MOUNT,
        sub = SCREENSHOTS_SUBDIR,
        out = out_path,
        url = url
    );

    let (logs, exit_code) = K8sManager::run_db_pvc_job(
        &client,
        &system_ns,
        &job_name,
        CHROME_IMAGE,
        vec![],
        &command,
        "hermes-backups",
    )
    .await?;

    if exit_code != 0 {
        let detail = logs
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        return Err(AppError::Infrastructure(format!(
            "Chromium screenshot job exited {} ({})",
            exit_code, detail
        )));
    }

    // The Job wrote to the shared PVC; the control plane sees the same path.
    if !Path::new(&out_path).exists() {
        return Err(AppError::Infrastructure(
            "Chromium job finished but produced no screenshot file.".to_string(),
        ));
    }

    sqlx::query(
        "UPDATE app_instances SET screenshot_path = $1, screenshot_captured_at = now() WHERE id = $2",
    )
    .bind(&out_path)
    .bind(instance_id)
    .execute(pool)
    .await?;

    Ok(())
}
