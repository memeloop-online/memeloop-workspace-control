use k8s_openapi::{
    api::core::v1::{ContainerStatus, Pod},
    jiff::Timestamp,
};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PodStartup {
    created_at: Option<String>,
    scheduled_at: Option<String>,
    containers_started_at: Option<String>,
    initialized_at: Option<String>,
    ready_at: Option<String>,
    total_seconds: Option<i64>,
    stages: Vec<StartupStage>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct StartupStage {
    name: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    duration_seconds: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct ContainerRuntime {
    name: String,
    ready: bool,
    restarts: i32,
    state: &'static str,
    reason: Option<String>,
    exit_code: Option<i32>,
}

fn condition(pod: &Pod, name: &str) -> Option<Timestamp> {
    pod.status
        .as_ref()?
        .conditions
        .as_ref()?
        .iter()
        .find(|condition| condition.type_ == name && condition.status == "True")?
        .last_transition_time
        .as_ref()
        .map(|time| time.0)
}

fn elapsed(start: Option<Timestamp>, finish: Option<Timestamp>) -> Option<i64> {
    Some(
        finish?
            .as_second()
            .saturating_sub(start?.as_second())
            .max(0),
    )
}

fn stage(name: &str, start: Option<Timestamp>, finish: Option<Timestamp>) -> StartupStage {
    StartupStage {
        name: name.to_owned(),
        started_at: start.map(|time| time.to_string()),
        finished_at: finish.map(|time| time.to_string()),
        duration_seconds: elapsed(start, finish),
    }
}

pub(super) fn pod_startup(pod: &Pod) -> PodStartup {
    let created = pod.metadata.creation_timestamp.as_ref().map(|time| time.0);
    let scheduled = condition(pod, "PodScheduled");
    let sandbox = condition(pod, "PodReadyToStartContainers");
    let initialized = condition(pod, "Initialized");
    let ready = condition(pod, "Ready");
    let mut stages = vec![
        stage("PodScheduled", created, scheduled),
        stage("PodReadyToStartContainers", scheduled, sandbox),
    ];
    for status in pod
        .status
        .as_ref()
        .and_then(|status| status.init_container_statuses.as_ref())
        .into_iter()
        .flatten()
    {
        let terminated = status
            .state
            .as_ref()
            .and_then(|state| state.terminated.as_ref());
        let start = terminated
            .and_then(|state| state.started_at.as_ref())
            .map(|time| time.0)
            .or_else(|| {
                status
                    .state
                    .as_ref()?
                    .running
                    .as_ref()?
                    .started_at
                    .as_ref()
                    .map(|time| time.0)
            });
        let finish = terminated
            .and_then(|state| state.finished_at.as_ref())
            .map(|time| time.0);
        stages.push(stage(&status.name, start, finish));
    }
    stages.push(stage("Ready", initialized, ready));
    PodStartup {
        created_at: created.map(|time| time.to_string()),
        scheduled_at: scheduled.map(|time| time.to_string()),
        containers_started_at: sandbox.map(|time| time.to_string()),
        initialized_at: initialized.map(|time| time.to_string()),
        ready_at: ready.map(|time| time.to_string()),
        total_seconds: elapsed(created, ready),
        stages,
    }
}

pub(super) fn container_runtime(status: &ContainerStatus) -> ContainerRuntime {
    let current = status.state.as_ref();
    let terminated = current
        .and_then(|state| state.terminated.as_ref())
        .or_else(|| status.last_state.as_ref()?.terminated.as_ref());
    let reason = current
        .and_then(|state| state.waiting.as_ref())
        .and_then(|state| state.reason.as_deref())
        .or_else(|| terminated.and_then(|state| state.reason.as_deref()));
    ContainerRuntime {
        name: status.name.clone(),
        ready: status.ready,
        restarts: status.restart_count,
        state: if current.is_some_and(|state| state.running.is_some()) {
            "running"
        } else if current.is_some_and(|state| state.terminated.is_some()) {
            "terminated"
        } else {
            "waiting"
        },
        reason: safe_reason(reason).map(str::to_owned),
        exit_code: terminated.map(|state| state.exit_code),
    }
}

/// Kubernetes reasons are extensible; only expose known codes, never messages.
pub(super) fn safe_reason(reason: Option<&str>) -> Option<&str> {
    reason.filter(|reason| {
        matches!(
            *reason,
            "BackOff"
                | "CrashLoopBackOff"
                | "ImagePullBackOff"
                | "ErrImagePull"
                | "FailedScheduling"
                | "Unhealthy"
                | "FailedMount"
                | "FailedAttachVolume"
                | "FailedCreatePodSandBox"
                | "OOMKilled"
                | "Error"
                | "Completed"
                | "ContainerCreating"
                | "PodInitializing"
                | "CreateContainerConfigError"
                | "CreateContainerError"
                | "RunContainerError"
                | "Evicted"
                | "Failed"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_exposes_storage_init_and_readiness_delays() {
        let pod: Pod = serde_json::from_value(serde_json::json!({
            "metadata":{"creationTimestamp":"2026-10-12T08:49:50Z"},
            "status":{"conditions":[
                {"type":"PodScheduled","status":"True","lastTransitionTime":"2026-10-12T08:50:21Z"},
                {"type":"PodReadyToStartContainers","status":"True","lastTransitionTime":"2026-10-12T08:50:39Z"},
                {"type":"Initialized","status":"True","lastTransitionTime":"2026-10-12T08:50:43Z"},
                {"type":"Ready","status":"True","lastTransitionTime":"2026-10-12T09:01:53Z"}
            ]}
        })).unwrap();
        let startup = pod_startup(&pod);
        assert_eq!(startup.total_seconds, Some(723));
        assert_eq!(startup.stages[0].duration_seconds, Some(31));
        assert_eq!(startup.stages[1].duration_seconds, Some(18));
        assert_eq!(startup.stages[2].duration_seconds, Some(670));
        assert!(safe_reason(Some("arbitrary sensitive diagnostics")).is_none());
    }

    #[test]
    fn a_pending_pod_has_no_invented_startup_duration() {
        let startup = pod_startup(&Pod::default());
        assert!(startup.total_seconds.is_none());
        assert!(
            startup
                .stages
                .iter()
                .all(|stage| stage.duration_seconds.is_none())
        );
    }
}
