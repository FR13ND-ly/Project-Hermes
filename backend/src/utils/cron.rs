use sqlx::PgPool;
use std::time::Duration;
use std::str::FromStr;
use uuid::Uuid;
use chrono::Utc;
use cron::Schedule;

use crate::models::app_model::AppStatus;
use crate::models::cron_model::{CronStatus, CronJobLog};
use crate::models::database_model::{DatabaseService, DbType};
use crate::models::user_model::UserStatus;
use crate::middlewares::auth_middleware::Claims;
use crate::utils::k8s::K8sManager;
use crate::utils::crypto;
use crate::utils::event_broadcaster::{broadcast_event, SystemEvent};

/// Full column list for `cron_jobs` selects (mirrors the CronJob model).
const CRON_COLS: &str = "id, workspace_id, project_id, app_id, target_type, target_id, is_backup, name, schedule, command, status, next_run_at, created_at, updated_at";

#[derive(sqlx::FromRow)]
struct SleepCandidate {
    id: uuid::Uuid,
    container_name: String,
    workspace_id: uuid::Uuid,
}

pub fn start_auto_sleep_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));

        loop {
            interval.tick().await;
            if !crate::utils::leader::is_leader() { continue; }

            let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
                Ok(c) => c,
                Err(_) => continue,
            };

            let inactive_instances = sqlx::query_as::<_, SleepCandidate>(
                "SELECT ai.id, ai.container_name, a.workspace_id FROM app_instances ai
                 JOIN apps a ON ai.app_id = a.id
                 WHERE ai.auto_sleep_enabled = true
                   AND ai.status = 'running'
                   AND ai.updated_at < now() - (ai.auto_sleep_after_minutes * interval '1 minute')"
            )
            .fetch_all(&pool)
            .await;

            if let Ok(instances) = inactive_instances {
                for inst in instances {
                    let k8s_client_clone = k8s_client.clone();
                    let pool_clone = pool.clone();
                    let container = inst.container_name.clone();
                    let namespace = format!("hermes-ws-{}", inst.workspace_id);
                    let inst_id = inst.id;
                    let workspace_id = inst.workspace_id;
                    tokio::spawn(async move {
                        if crate::utils::k8s::K8sManager::scale_deployment(&k8s_client_clone, &namespace, &container, 0).await.is_ok() {
                            let _ = sqlx::query!(
                                "UPDATE app_instances SET status = $1, updated_at = now() WHERE id = $2",
                                AppStatus::Stopped as AppStatus, inst_id
                            )
                            .execute(&pool_clone)
                            .await;

                            crate::utils::event_broadcaster::broadcast_event(
                                crate::utils::event_broadcaster::SystemEvent::InstanceStatusChanged {
                                    workspace_id,
                                    instance_id: inst_id,
                                    container_name: container.clone(),
                                    status: "stopped".to_string(),
                                }
                            );

                            tracing::info!(%container, %workspace_id, "Auto-sleep: scaled deployment to 0 replicas");
                        }
                    });
                }
            }
        }
    });
}

/// Reconcile managed-backup crons: ensure every backup-enabled database has one.
/// Runs once at startup so existing databases (enabled before this feature) get a
/// real, visible backup cron.
pub async fn reconcile_backup_crons(pool: &PgPool) {
    if let Ok(rows) = sqlx::query!("SELECT id FROM databases WHERE backup_enabled = true").fetch_all(pool).await {
        for r in rows {
            let _ = crate::controllers::cron_controller::ensure_backup_cron(pool, r.id).await;
        }
    }
}

pub fn start_cron_scheduler_engine(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));

        loop {
            interval.tick().await;
            if !crate::utils::leader::is_leader() { continue; }

            let now = Utc::now();
            let executable_jobs = sqlx::query!(
                "SELECT id, workspace_id, target_type, target_id, is_backup, name, schedule, command
                 FROM cron_jobs
                 WHERE status = 'active' AND next_run_at <= $1",
                now
            )
            .fetch_all(&pool)
            .await;

            if let Ok(jobs) = executable_jobs {
                for job in jobs {
                    tracing::info!(job = %job.name, job_id = %job.id, "Cron match: spawning execution");

                    // Advance next_run_at synchronously so the job isn't re-picked.
                    if let Ok(sched) = Schedule::from_str(&job.schedule) {
                        let next_run = sched.upcoming(Utc).next().map(|dt| dt.with_timezone(&Utc));
                        let _ = sqlx::query!("UPDATE cron_jobs SET next_run_at = $1, updated_at = now() WHERE id = $2", next_run, job.id).execute(&pool).await;

                        if let Ok(updated_job) = sqlx::query_as::<_, crate::models::cron_model::CronJob>(
                            &format!("SELECT {CRON_COLS} FROM cron_jobs WHERE id = $1")
                        )
                        .bind(job.id)
                        .fetch_one(&pool)
                        .await {
                            broadcast_event(SystemEvent::CronJobUpdated { workspace_id: job.workspace_id, job: updated_job });
                        }
                    }

                    let pool_execution = pool.clone();
                    let target_type = job.target_type.clone();
                    let target_id = job.target_id;
                    let is_backup = job.is_backup;
                    let command = job.command.clone();
                    let job_id = job.id;
                    let ws = job.workspace_id;
                    tokio::spawn(async move {
                        let _ = execute_cron_job(pool_execution, job_id, ws, target_type, target_id, is_backup, command).await;
                    });
                }
            }
        }
    });
}

/// Insert a cron log row and broadcast it.
async fn log_cron(pool: &PgPool, job_id: Uuid, workspace_id: Uuid, exit_code: i32, output: &str, started_at: chrono::DateTime<Utc>) {
    let log_id = Uuid::new_v4();
    let finished_at = Utc::now();

    // Every cron execution path funnels through here, so it's the single point
    // to record run telemetry (exit_code 0 = success).
    let duration_secs = (finished_at - started_at).num_milliseconds().max(0) as f64 / 1000.0;
    crate::utils::metrics::record_cron_run(
        if exit_code == 0 { "success" } else { "failed" },
        duration_secs,
    );
    let _ = sqlx::query!(
        "INSERT INTO cron_job_logs (id, cron_job_id, exit_code, output, started_at, finished_at) VALUES ($1, $2, $3, $4, $5, $6)",
        log_id, job_id, exit_code, Some(output.to_string()), started_at, finished_at
    )
    .execute(pool)
    .await;

    broadcast_event(SystemEvent::CronJobLogCreated {
        workspace_id,
        job_id,
        log: CronJobLog {
            id: log_id,
            cron_job_id: job_id,
            exit_code,
            output: Some(output.to_string()),
            started_at,
            finished_at,
        },
    });
}

/// Run a one-shot K8s Job for the cron command and log its output.
async fn run_k8s_job(
    pool: &PgPool,
    job_id: Uuid,
    workspace_id: Uuid,
    namespace: String,
    image: String,
    env: Vec<(String, String)>,
    command: String,
    started_at: chrono::DateTime<Utc>,
) -> Result<(), ()> {
    let client = match K8sManager::get_client().await {
        Ok(c) => c,
        Err(e) => {
            log_cron(pool, job_id, workspace_id, 1, &format!("Eroare conectare Kubernetes: {}", e), started_at).await;
            return Err(());
        }
    };
    let job_name = format!("hermes-cron-{}-{}", &job_id.to_string()[..18], Utc::now().timestamp()).to_lowercase();

    match K8sManager::run_job_and_get_logs(&client, &namespace, &job_name, &image, env, &command).await {
        Ok((output, exit_code)) => {
            log_cron(pool, job_id, workspace_id, exit_code, &output, started_at).await;
            Ok(())
        }
        Err(e) => {
            log_cron(pool, job_id, workspace_id, 1, &format!("Eroare la rularea containerului în Kubernetes: {:?}", e), started_at).await;
            Err(())
        }
    }
}

/// Dispatch a cron run based on its target (app / database / storage).
async fn execute_cron_job(
    pool: PgPool,
    job_id: Uuid,
    workspace_id: Uuid,
    target_type: String,
    target_id: Option<Uuid>,
    is_backup: bool,
    command: String,
) -> Result<(), ()> {
    let started_at = Utc::now();

    let Some(target_id) = target_id else {
        log_cron(&pool, job_id, workspace_id, 1, "Eroare: cron-ul nu are o resursă țintă.", started_at).await;
        return Err(());
    };

    match target_type.as_str() {
        // Managed database backup: preserves file storage + retention + restore.
        "database" if is_backup => {
            match crate::controllers::database_controller::perform_database_backup(&pool, target_id, Some(&command)).await {
                Ok(res) => {
                    let kb = (res.file_size_bytes as f64 / 1024.0).max(0.0);
                    let mut msg = format!("📦 Backup creat: {} · {:.1} KB", res.filename, kb);
                    if let Some(extra) = res.log.as_ref().filter(|s| !s.is_empty()) {
                        msg.push('\n');
                        msg.push_str(extra);
                    }
                    log_cron(&pool, job_id, workspace_id, 0, &msg, started_at).await;
                    Ok(())
                }
                Err(e) => {
                    log_cron(&pool, job_id, workspace_id, 1, &format!("Backup eșuat: {:?}", e), started_at).await;
                    Err(())
                }
            }
        }
        // The cron's own env (custom vars + linked project-pool vars) is merged into
        // every non-backup run, overriding the target's base/connection vars on a key clash.
        "app" => {
            let extra = crate::utils::app_env::resolve_cron_env(&pool, job_id).await;
            run_app_cron(&pool, job_id, workspace_id, target_id, command, extra, started_at).await
        }
        "database" => {
            let extra = crate::utils::app_env::resolve_cron_env(&pool, job_id).await;
            run_database_cron(&pool, job_id, workspace_id, target_id, command, extra, started_at).await
        }
        "storage" => {
            let extra = crate::utils::app_env::resolve_cron_env(&pool, job_id).await;
            run_storage_cron(&pool, job_id, workspace_id, target_id, command, extra, started_at).await
        }
        other => {
            log_cron(&pool, job_id, workspace_id, 1, &format!("Tip de țintă necunoscut: {}", other), started_at).await;
            Err(())
        }
    }
}

/// Merge a base env vec with the cron's own env, the cron's value winning on a key
/// conflict. Dedupes by key (K8s rejects a container with duplicate env names).
fn merge_env(base: Vec<(String, String)>, extra: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut map: std::collections::HashMap<String, String> = base.into_iter().collect();
    for (k, v) in extra {
        map.insert(k, v);
    }
    let mut out: Vec<(String, String)> = map.into_iter().collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// App cron: runs the command in the app's production image with its effective env.
async fn run_app_cron(pool: &PgPool, job_id: Uuid, workspace_id: Uuid, app_id: Uuid, command: String, extra_env: Vec<(String, String)>, started_at: chrono::DateTime<Utc>) -> Result<(), ()> {
    let inst = sqlx::query!(
        "SELECT id FROM app_instances WHERE app_id = $1 AND instance_type = 'production'",
        app_id
    )
    .fetch_optional(pool)
    .await;

    let inst_id = match inst {
        Ok(Some(r)) => r.id,
        _ => {
            log_cron(pool, job_id, workspace_id, 1, "Aplicația nu are o instanță de producție. Cron-ul are nevoie de o imagine creată la deploy-ul de producție.", started_at).await;
            return Err(());
        }
    };

    let image = crate::utils::builder::resolve_instance_image_tag(pool, inst_id).await;
    let env = merge_env(crate::utils::app_env::resolve_instance_env(pool, inst_id).await, extra_env);
    let namespace = format!("hermes-ws-{}", workspace_id);
    run_k8s_job(pool, job_id, workspace_id, namespace, image, env, command, started_at).await
}

/// Database cron (non-backup): runs on the DB image with connection vars injected.
async fn run_database_cron(pool: &PgPool, job_id: Uuid, workspace_id: Uuid, db_id: Uuid, command: String, extra_env: Vec<(String, String)>, started_at: chrono::DateTime<Utc>) -> Result<(), ()> {
    let db = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1")
        .bind(db_id)
        .fetch_optional(pool)
        .await;
    let db = match db {
        Ok(Some(d)) => d,
        _ => {
            log_cron(pool, job_id, workspace_id, 1, "Baza de date țintă nu a fost găsită.", started_at).await;
            return Err(());
        }
    };

    let password = match &db.db_password_nonce {
        Some(nonce) => crypto::decrypt_env_value(&db.db_password, nonce).unwrap_or_default(),
        None => crypto::decrypt_env_value(&db.db_password, "AAAAAAAAAAAAAAAA").unwrap_or_default(),
    };

    let type_str = match db.r#type {
        DbType::Postgres => "postgres",
        DbType::Mysql => "mysql",
        DbType::Redis => "redis",
        DbType::Mongodb => "mongodb",
    };
    let image = format!("{}:{}", type_str, db.version);

    let url = match db.r#type {
        DbType::Postgres => format!("postgresql://{}:{}@{}:{}/{}", db.db_user, password, db.container_name, db.internal_port, db.db_name),
        DbType::Mysql => format!("mysql://{}:{}@{}:{}/{}", db.db_user, password, db.container_name, db.internal_port, db.db_name),
        DbType::Redis => format!("redis://{}:{}", db.container_name, db.internal_port),
        DbType::Mongodb => format!("mongodb://{}:{}@{}:{}", db.db_user, password, db.container_name, db.internal_port),
    };

    let env = merge_env(vec![
        ("DATABASE_URL".to_string(), url),
        ("DB_HOST".to_string(), db.container_name.clone()),
        ("DB_PORT".to_string(), db.internal_port.to_string()),
        ("DB_USER".to_string(), db.db_user.clone()),
        ("DB_PASSWORD".to_string(), password),
        ("DB_NAME".to_string(), db.db_name.clone()),
    ], extra_env);
    let namespace = format!("hermes-ws-{}", db.workspace_id);
    run_k8s_job(pool, job_id, workspace_id, namespace, image, env, command, started_at).await
}

/// Storage cron: runs on a small base image (curl) with the bucket URL + a freshly
/// minted access token injected, so the command can hit the bucket's API.
async fn run_storage_cron(pool: &PgPool, job_id: Uuid, workspace_id: Uuid, bucket_id: Uuid, command: String, extra_env: Vec<(String, String)>, started_at: chrono::DateTime<Utc>) -> Result<(), ()> {
    let bucket = sqlx::query!(
        "SELECT workspace_id, slug, created_by FROM storage_buckets WHERE id = $1",
        bucket_id
    )
    .fetch_optional(pool)
    .await;
    let bucket = match bucket {
        Ok(Some(b)) => b,
        _ => {
            log_cron(pool, job_id, workspace_id, 1, "Bucket-ul de storage țintă nu a fost găsit.", started_at).await;
            return Err(());
        }
    };

    let base_domain = std::env::var("HERMES_BASE_DOMAIN").unwrap_or_else(|_| "hermes-host.vip".to_string());
    let bucket_url = format!("https://{}.{}", bucket.slug, base_domain);

    // Mint a long-ish-lived access token scoped to the bucket's workspace.
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "super_secret_key".to_string());
    let exp = Utc::now() + chrono::Duration::days(1);
    let claims = Claims {
        sub: bucket.created_by,
        username: "hermes-cron".to_string(),
        email: "cron@hermes.local".to_string(),
        status: UserStatus::Active,
        is_super_admin: true,
        current_workspace_id: Some(bucket.workspace_id),
        exp: exp.timestamp(),
    };
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.as_bytes()),
    )
    .unwrap_or_default();

    let env = merge_env(vec![
        ("BUCKET_URL".to_string(), bucket_url),
        ("BUCKET_SLUG".to_string(), bucket.slug.clone()),
        ("BUCKET_TOKEN".to_string(), token),
    ], extra_env);
    let namespace = format!("hermes-ws-{}", bucket.workspace_id);
    run_k8s_job(pool, job_id, workspace_id, namespace, "curlimages/curl:latest".to_string(), env, command, started_at).await
}
