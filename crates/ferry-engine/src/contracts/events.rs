// 此文件由 scripts/generate-contracts.py 生成，请勿手改。

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventSource {
    Runtime,
    Host,
    Engine,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventPolicy {
    pub source: EventSource,
    pub forward_to_ui: bool,
}

pub const FERRY_EVENT_TYPES: &[&str] = &[
    "auth.cancelled",
    "auth.completed",
    "auth.event",
    "auth.failed",
    "auth.prompt",
    "choice.requested",
    "choice.resolved",
    "content.delta",
    "engine.request",
    "operation.applied",
    "operation.failed",
    "operation.proposed",
    "run.cancelled",
    "run.completed",
    "run.failed",
    "run.interrupted",
    "run.started",
    "runtime.disconnected",
    "session.created",
    "session.model_changed",
    "session.renamed",
    "sessions.changed",
    "tool.completed",
    "tool.progress",
    "tool.request",
    "tool.started",
    "user.message",
];

pub fn event_policy(event_type: &str) -> Option<EventPolicy> {
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
        "choice.requested" => Some(EventPolicy {
            source: EventSource::Host,
            forward_to_ui: true,
        }),
        "choice.resolved" => Some(EventPolicy {
            source: EventSource::Host,
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
        "session.renamed" => Some(EventPolicy {
            source: EventSource::Runtime,
            forward_to_ui: true,
        }),
        "sessions.changed" => Some(EventPolicy {
            source: EventSource::Engine,
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
        _ => None,
    }
}
