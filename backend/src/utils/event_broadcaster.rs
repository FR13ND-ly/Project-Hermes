use std::sync::OnceLock;
use tokio::sync::broadcast;
use uuid::Uuid;
use serde::Serialize;

#[derive(Clone, Serialize, Debug)]
#[serde(tag = "type", content = "payload")]
#[serde(rename_all = "snake_case")]
pub enum SystemEvent {
    InstanceStatusChanged {
        workspace_id: Uuid,
        instance_id: Uuid,
        container_name: String,
        status: String,
    },
    DatabaseStatusChanged {
        workspace_id: Uuid,
        database_id: Uuid,
        container_name: String,
        status: String,
    },
    BuildStatusChanged {
        workspace_id: Uuid,
        build_id: Uuid,
        app_id: Uuid,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
    },
    IncidentCreated {
        workspace_id: Uuid,
        incident_id: Uuid,
        project_id: Uuid,
        message: String,
    },
    CronJobUpdated {
        workspace_id: Uuid,
        job: crate::models::cron_model::CronJob,
    },
    CronJobDeleted {
        workspace_id: Uuid,
        job_id: Uuid,
    },
    CronJobLogCreated {
        workspace_id: Uuid,
        job_id: Uuid,
        log: crate::models::cron_model::CronJobLog,
    },
    ServerlessFunctionUpdated {
        workspace_id: Uuid,
        instance: crate::models::serverless_model::ServerlessInstance,
    },
    ServerlessFunctionDeleted {
        workspace_id: Uuid,
        instance_id: Uuid,
    },
    StorageObjectUpdated {
        workspace_id: Uuid,
        bucket_id: Uuid,
        object_id: Uuid,
        status: String,
    },
}

static WS_SENDER: OnceLock<broadcast::Sender<SystemEvent>> = OnceLock::new();

pub fn get_ws_sender() -> &'static broadcast::Sender<SystemEvent> {
    WS_SENDER.get_or_init(|| {
        let (tx, _) = broadcast::channel(2048);
        tx
    })
}

pub fn broadcast_event(event: SystemEvent) {
    let _ = get_ws_sender().send(event);
}
