// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EventSource {
    Runtime,
    Host,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EventPolicy {
    pub(crate) source: EventSource,
    pub(crate) forward_to_ui: bool,
}

pub(crate) fn event_policy(event_type: &str) -> Option<EventPolicy> {
    match event_type {
        "auth.cancelled" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "auth.completed" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "auth.event" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "auth.failed" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "auth.prompt" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "content.delta" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "engine.request" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: false,
        }),
        "operation.applied" => Some(EventPolicy {
            source: EventSource::Host,
            forward_to_ui: true,
        }),
        "operation.failed" => Some(EventPolicy {
            source: EventSource::Host,
            forward_to_ui: true,
        }),
        "operation.proposed" => Some(EventPolicy {
            source: EventSource::Host,
            forward_to_ui: true,
        }),
        "run.cancelled" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "run.completed" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "run.failed" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "run.interrupted" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "run.started" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "runtime.disconnected" => Some(EventPolicy {
            source: EventSource::Host,
            forward_to_ui: true,
        }),
        "session.created" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "session.model_changed" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "task.cancelled" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "task.completed" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "task.failed" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "task.skipped" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "task.started" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "tool.completed" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "tool.progress" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "tool.request" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "tool.started" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "user.message" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "workflow.completed" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "workflow.started" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        _ => None,
    }
}
