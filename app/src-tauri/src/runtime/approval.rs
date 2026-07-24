use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static AUTO_SESSIONS: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

pub(super) fn remember_auto_policy(request: &str) {
    let Ok(value) = serde_json::from_str::<Value>(request) else {
        return;
    };
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if method != "prompt" {
        return;
    }
    let Some(params) = value.get("params") else {
        return;
    };
    let Some(session_id) = params.get("session_id").and_then(Value::as_str) else {
        return;
    };
    let auto = params
        .get("auto_apply")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Ok(mut sessions) = AUTO_SESSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        sessions.insert(session_id.to_owned(), auto);
    }
}

pub(super) fn auto_policy(session_id: &str) -> bool {
    AUTO_SESSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|sessions| sessions.get(session_id).copied())
        .unwrap_or(false)
}

pub(super) fn forget_auto_policy(session_id: &str) {
    if let Ok(mut sessions) = AUTO_SESSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        sessions.remove(session_id);
    }
}

pub(super) fn allows_auto_apply(prompt_auto: bool, role_policy: &str) -> bool {
    prompt_auto && role_policy == "auto"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn automatic_apply_policy_is_bound_to_the_prompt_run() {
        let session_id = "test-auto-policy-prompt-run";
        forget_auto_policy(session_id);
        remember_auto_policy(
            &json!({"method": "prompt", "params": {
                "session_id": session_id, "auto_apply": true
            }})
            .to_string(),
        );
        assert!(auto_policy(session_id));

        remember_auto_policy(
            &json!({"method": "follow_up", "params": {
                "session_id": session_id, "auto_apply": false
            }})
            .to_string(),
        );
        assert!(auto_policy(session_id));
        forget_auto_policy(session_id);
        assert!(!auto_policy(session_id));
    }

    #[test]
    fn role_manual_policy_cannot_be_overridden_by_prompt_auto_mode() {
        assert!(!allows_auto_apply(true, "manual"));
        assert!(allows_auto_apply(true, "auto"));
        assert!(!allows_auto_apply(false, "auto"));
    }
}
