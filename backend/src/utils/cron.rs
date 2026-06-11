use sqlx::PgPool;
use std::time::Duration;
use std::str::FromStr;
use uuid::Uuid;
use chrono::Utc;
use cron::Schedule;

use crate::models::app_model::AppStatus;
use crate::models::cron_model::CronStatus;

pub fn start_auto_sleep_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        
        loop {
            interval.tick().await;
            
            let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
                Ok(c) => c,
                Err(_) => continue,
            };

            let inactive_instances = sqlx::query!(
                "SELECT ai.id, ai.container_name, a.workspace_id FROM app_instances ai
                 JOIN apps a ON ai.app_id = a.id
                 WHERE ai.instance_type != 'production' 
                   AND ai.status = 'running' 
                   AND ai.updated_at < now() - interval '30 minutes'"
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
                            
                            println!("[Hermes Auto-Sleep] Deployment scaled to 0 replicas: {}", container);
                        }
                    });                }
            }
        }
    });
}

pub fn start_cron_scheduler_engine(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        
        loop {
            interval.tick().await;
            
            let now = Utc::now();
            let executable_jobs = sqlx::query!(
                "SELECT id, workspace_id, project_id, app_id, name, schedule, command, status as \"status: CronStatus\", next_run_at, created_at, updated_at 
                 FROM cron_jobs 
                 WHERE status = 'active' AND next_run_at <= $1",
                now
            )
            .fetch_all(&pool)
            .await;

            if let Ok(jobs) = executable_jobs {
                for job in jobs {
                    let schedule_str = job.schedule.clone();
                    
                    println!("[Cron Scheduler] Match found! Spawning execution task for job: {} (ID: {})", job.name, job.id);
                    
                    // Update next_run_at SYNCHRONOUSLY before spawning execution
                    // to prevent the same job from being picked up on the next tick
                    if let Ok(sched) = Schedule::from_str(&schedule_str) {
                        let next_run = sched.upcoming(Utc).next().map(|dt| dt.with_timezone(&Utc));
                        let _ = sqlx::query!("UPDATE cron_jobs SET next_run_at = $1, updated_at = now() WHERE id = $2", next_run, job.id).execute(&pool).await;
                        
                        // Fetch the updated job and broadcast the change
                        if let Ok(updated_job) = sqlx::query_as::<_, crate::models::cron_model::CronJob>(
                            "SELECT id, workspace_id, project_id, app_id, name, schedule, command, status, next_run_at, created_at, updated_at 
                             FROM cron_jobs 
                             WHERE id = $1"
                        )
                        .bind(job.id)
                        .fetch_one(&pool)
                        .await {
                            crate::utils::event_broadcaster::broadcast_event(
                                crate::utils::event_broadcaster::SystemEvent::CronJobUpdated {
                                    workspace_id: job.workspace_id,
                                    job: updated_job,
                                }
                            );
                        }
                    }

                    let pool_execution = pool.clone();
                    tokio::spawn(async move {
                        let _ = execute_cron_container(pool_execution, job.id, job.app_id, job.command).await;
                    });
                }
            }
        }
    });
}

async fn execute_cron_container(pool: PgPool, job_id: Uuid, app_id: Uuid, command: String) -> Result<(), ()> {
    let started_at = Utc::now();
    println!("[Cron Runner] Starting execute_cron_container for job_id={} app_id={}", job_id, app_id);

    let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
        Ok(c) => c,
        Err(e) => {
            let _ = sqlx::query!(
                "INSERT INTO cron_job_logs (id, cron_job_id, exit_code, output, started_at, finished_at) VALUES ($1, $2, $3, $4, $5, $6)",
                Uuid::new_v4(), job_id, 1, Some(format!("Eroare conectare Kubernetes: {}", e)), started_at, Utc::now()
            )
            .execute(&pool)
            .await;
            return Err(());
        }
    };

    let app_meta = match sqlx::query!(
        "SELECT workspace_id, project_id FROM apps WHERE id = $1",
        app_id
    )
    .fetch_optional(&pool)
    .await {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            let _ = sqlx::query!(
                "INSERT INTO cron_job_logs (id, cron_job_id, exit_code, output, started_at, finished_at) VALUES ($1, $2, $3, $4, $5, $6)",
                Uuid::new_v4(), job_id, 1, Some("Eroare: Aplicația targetată nu a fost găsită în baza de date.".to_string()), started_at, Utc::now()
            )
            .execute(&pool)
            .await;
            return Err(());
        }
        Err(e) => {
            let _ = sqlx::query!(
                "INSERT INTO cron_job_logs (id, cron_job_id, exit_code, output, started_at, finished_at) VALUES ($1, $2, $3, $4, $5, $6)",
                Uuid::new_v4(), job_id, 1, Some(format!("Eroare bază de date la căutarea aplicației: {}", e)), started_at, Utc::now()
            )
            .execute(&pool)
            .await;
            return Err(());
        }
    };
    let namespace = format!("hermes-ws-{}", app_meta.workspace_id);

    // Get the production instance ID to determine the image tag
    let inst_meta = match sqlx::query!(
        "SELECT id FROM app_instances WHERE app_id = $1 AND instance_type = 'production'",
        app_id
    )
    .fetch_optional(&pool)
    .await {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            let _ = sqlx::query!(
                "INSERT INTO cron_job_logs (id, cron_job_id, exit_code, output, started_at, finished_at) VALUES ($1, $2, $3, $4, $5, $6)",
                Uuid::new_v4(), job_id, 1, Some("Eroare: Aplicația nu are nicio instanță de producție activă. Sarcina cron are nevoie de o imagine de container creată la deploy-ul de producție pentru a rula comanda.".to_string()), started_at, Utc::now()
            )
            .execute(&pool)
            .await;
            return Err(());
        }
        Err(e) => {
            let _ = sqlx::query!(
                "INSERT INTO cron_job_logs (id, cron_job_id, exit_code, output, started_at, finished_at) VALUES ($1, $2, $3, $4, $5, $6)",
                Uuid::new_v4(), job_id, 1, Some(format!("Eroare bază de date la căutarea instanței: {}", e)), started_at, Utc::now()
            )
            .execute(&pool)
            .await;
            return Err(());
        }
    };
    
    let image_tag = crate::utils::builder::resolve_instance_image_tag(&pool, inst_meta.id).await;
    let job_name = format!(
        "hermes-cron-{}-{}",
        &job_id.to_string()[..18],
        Utc::now().timestamp()
    ).to_lowercase();

    let mut env_variables = Vec::new();
    let env_records = sqlx::query!(
        "SELECT key, encrypted_value, nonce FROM environment_variables
         WHERE app_instance_id = $1",
        inst_meta.id
    )
    .fetch_all(&pool)
    .await;

    if let Ok(records) = env_records {
        for rec in records {
            if let Ok(dec_val) = crate::utils::crypto::decrypt_env_value(&rec.encrypted_value, &rec.nonce) {
                env_variables.push((rec.key, dec_val));
            }
        }
    }

    println!("[Cron Runner] Calling run_job_and_get_logs for job_id={} namespace={} job_name={} image_tag={} command={}", job_id, namespace, job_name, image_tag, command);
    let run_result = crate::utils::k8s::K8sManager::run_job_and_get_logs(
        &k8s_client,
        &namespace,
        &job_name,
        &image_tag,
        env_variables,
        &command,
    ).await;

    println!("[Cron Runner] run_job_and_get_logs completed for job_id={}. Result: {:?}", job_id, run_result.as_ref().map(|(logs, exit)| (logs.len(), *exit)));

    let (output_accumulator, exit_code) = match run_result {
        Ok(res) => res,
        Err(e) => {
            println!("[Cron Runner] Kubernetes job failed for job_id={}: {:?}", job_id, e);
            let log_id = Uuid::new_v4();
            let finished_at = Utc::now();
            let err_msg = format!("Eroare la rularea containerului în Kubernetes: {:?}", e);
            let _ = sqlx::query!(
                "INSERT INTO cron_job_logs (id, cron_job_id, exit_code, output, started_at, finished_at) VALUES ($1, $2, $3, $4, $5, $6)",
                log_id, job_id, 1, Some(&err_msg), started_at, finished_at
            )
            .execute(&pool)
            .await;
            
            crate::utils::event_broadcaster::broadcast_event(
                crate::utils::event_broadcaster::SystemEvent::CronJobLogCreated {
                    workspace_id: app_meta.workspace_id,
                    job_id,
                    log: crate::models::cron_model::CronJobLog {
                        id: log_id,
                        cron_job_id: job_id,
                        exit_code: 1,
                        output: Some(err_msg),
                        started_at,
                        finished_at,
                    }
                }
            );
            return Err(());
        }
    };

    println!("[Cron Runner] Inserting log row into database for job_id={}", job_id);
    let log_id = Uuid::new_v4();
    let finished_at = Utc::now();
    let db_res = sqlx::query!(
        "INSERT INTO cron_job_logs (id, cron_job_id, exit_code, output, started_at, finished_at) VALUES ($1, $2, $3, $4, $5, $6)",
        log_id, job_id, exit_code, Some(&output_accumulator), started_at, finished_at
    )
    .execute(&pool)
    .await;
    println!("[Cron Runner] Database log row insertion completed for job_id={}. Result: {:?}", job_id, db_res);

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::CronJobLogCreated {
            workspace_id: app_meta.workspace_id,
            job_id,
            log: crate::models::cron_model::CronJobLog {
                id: log_id,
                cron_job_id: job_id,
                exit_code,
                output: Some(output_accumulator),
                started_at,
                finished_at,
            }
        }
    );

    Ok(())
}

pub fn start_auto_backup_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        
        loop {
            interval.tick().await;
            
            let query_res = sqlx::query!(
                "SELECT id FROM databases 
                 WHERE backup_enabled = true 
                   AND (last_backup_at IS NULL OR last_backup_at < now() - interval '24 hours')"
            )
            .fetch_all(&pool)
            .await;

            if let Ok(dbs) = query_res {
                for db in dbs {
                    let pool_clone = pool.clone();
                    let db_id = db.id;
                    tokio::spawn(async move {
                        println!("[Auto Backup Worker] Triggering automatic backup for database: {}", db_id);
                        match crate::controllers::database_controller::perform_database_backup(&pool_clone, db_id).await {
                            Ok(res) => {
                                println!("[Auto Backup Worker] Backup completed successfully for db={}: filename={}", db_id, res.filename);
                            }
                            Err(e) => {
                                eprintln!("[Auto Backup Worker] Backup failed for db={}: {:?}", db_id, e);
                            }
                        }
                    });
                }
            }
        }
    });
}