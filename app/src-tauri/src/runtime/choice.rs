//! ask_user 的宿主侧挂起、应答与会话生命周期管理。

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::AppHandle;

use crate::contracts::ipc::FERRY_IPC_PROTOCOL;

use super::emit_host_event;

const CHOICE_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

struct PendingChoice {
    session_id: String,
    sender: Sender<Value>,
}

static PENDING: OnceLock<Mutex<HashMap<String, PendingChoice>>> = OnceLock::new();

fn pending() -> &'static Mutex<HashMap<String, PendingChoice>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fallback_answer() -> Value {
    json!({"answered": false, "selected": []})
}

fn invalid_args() -> String {
    "tool.invalid_args".to_owned()
}

fn validate_args(args: &Map<String, Value>) -> Result<(String, Value, bool, bool), String> {
    let question = args
        .get("question")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.chars().count() <= 500)
        .ok_or_else(invalid_args)?
        .to_owned();
    let options = args
        .get("options")
        .and_then(Value::as_array)
        .filter(|options| (2..=6).contains(&options.len()))
        .ok_or_else(invalid_args)?;
    let mut labels = std::collections::HashSet::new();
    for option in options {
        let object = option.as_object().ok_or_else(invalid_args)?;
        let label = object
            .get("label")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.chars().count() <= 80)
            .ok_or_else(invalid_args)?;
        if !labels.insert(label.to_owned()) {
            return Err(invalid_args());
        }
        if object
            .get("description")
            .is_some_and(|value| value.as_str().is_none_or(|text| text.chars().count() > 200))
            || object
                .get("recommended")
                .is_some_and(|value| !value.is_boolean())
        {
            return Err(invalid_args());
        }
    }
    let multi_select = match args.get("multi_select") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(invalid_args)?,
    };
    let allow_custom = match args.get("allow_custom") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(invalid_args)?,
    };
    Ok((
        question,
        Value::Array(options.clone()),
        multi_select,
        allow_custom,
    ))
}

fn validate_answer(answer: &Value) -> Result<(), String> {
    let object = answer
        .as_object()
        .ok_or_else(|| "choice.invalid_answer".to_owned())?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "answered" | "selected" | "custom_text"))
        || object.get("answered").and_then(Value::as_bool).is_none()
        || object
            .get("selected")
            .and_then(Value::as_array)
            .is_none_or(|selected| !selected.iter().all(Value::is_string))
        || object
            .get("custom_text")
            .is_some_and(|custom_text| !custom_text.is_string())
    {
        return Err("choice.invalid_answer".to_owned());
    }
    Ok(())
}

fn register_pending(request_id: &str, session_id: &str) -> Result<Receiver<Value>, String> {
    let (sender, receiver) = mpsc::channel();
    let mut entries = pending().lock().map_err(|_| "internal_error".to_owned())?;
    if entries.contains_key(request_id) {
        return Err("choice.request_id_in_use".to_owned());
    }
    entries.insert(
        request_id.to_owned(),
        PendingChoice {
            session_id: session_id.to_owned(),
            sender,
        },
    );
    Ok(receiver)
}

fn remove_pending(request_id: &str) {
    if let Ok(mut entries) = pending().lock() {
        entries.remove(request_id);
    }
}

fn respond_value(request_id: &str, answer: Value) -> Result<(), String> {
    validate_answer(&answer)?;
    let entry = pending()
        .lock()
        .map_err(|_| "internal_error".to_owned())?
        .remove(request_id)
        .ok_or_else(|| "choice.request_not_found".to_owned())?;
    entry
        .sender
        .send(answer)
        .map_err(|_| "choice.request_closed".to_owned())
}

fn wait_for_answer(request_id: &str, receiver: Receiver<Value>) -> Value {
    match receiver.recv_timeout(CHOICE_TIMEOUT) {
        Ok(answer) => answer,
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            remove_pending(request_id);
            fallback_answer()
        }
    }
}

fn answer_payload(request_id: &str, answer: &Value) -> Value {
    let mut payload = Map::new();
    payload.insert(
        "request_id".to_owned(),
        Value::String(request_id.to_owned()),
    );
    if let Some(object) = answer.as_object() {
        for key in ["answered", "selected", "custom_text"] {
            if let Some(value) = object.get(key) {
                payload.insert(key.to_owned(), value.clone());
            }
        }
    }
    Value::Object(payload)
}

pub(super) fn propose(
    app: &AppHandle,
    session_id: &str,
    run_id: &str,
    request_id: &str,
    tool_call_id: &str,
    args: &Map<String, Value>,
) -> Result<Value, String> {
    let (question, options, multi_select, allow_custom) = validate_args(args)?;
    let receiver = register_pending(request_id, session_id)?;
    emit_host_event(
        app,
        json!({
            "protocol": FERRY_IPC_PROTOCOL,
            "session_id": session_id,
            "run_id": run_id,
            "type": "choice.requested",
            "payload": {
                "request_id": request_id,
                "tool_call_id": tool_call_id,
                "question": question,
                "options": options,
                "multi_select": multi_select,
                "allow_custom": allow_custom,
            },
        }),
    );
    let answer = wait_for_answer(request_id, receiver);
    emit_host_event(
        app,
        json!({
            "protocol": FERRY_IPC_PROTOCOL,
            "session_id": session_id,
            "run_id": run_id,
            "type": "choice.resolved",
            "payload": answer_payload(request_id, &answer),
        }),
    );
    Ok(answer)
}

#[tauri::command]
pub(crate) fn choice_respond(request_id: String, answer: Value) -> Result<(), String> {
    respond_value(&request_id, answer)
}

pub(super) fn cancel_session(session_id: &str) {
    let entries = match pending().lock() {
        Ok(mut entries) => {
            let request_ids: Vec<String> = entries
                .iter()
                .filter(|(_, entry)| entry.session_id == session_id)
                .map(|(request_id, _)| request_id.clone())
                .collect();
            request_ids
                .into_iter()
                .filter_map(|request_id| entries.remove(&request_id))
                .collect::<Vec<_>>()
        }
        Err(_) => return,
    };
    for entry in entries {
        let _ = entry.sender.send(fallback_answer());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn wait(request_id: &str, receiver: Receiver<Value>) -> Value {
        wait_for_answer(request_id, receiver)
    }

    #[test]
    fn respond_wakes_waiter_and_returns_original_value() {
        let request_id = "choice-test-response";
        remove_pending(request_id);
        let receiver = register_pending(request_id, "session-response").unwrap();
        let waiter = thread::spawn(move || wait(request_id, receiver));
        let answer = json!({"answered": true, "selected": ["keep"]});
        respond_value(request_id, answer.clone()).unwrap();
        assert_eq!(waiter.join().unwrap(), answer);
    }

    #[test]
    fn unknown_request_id_is_rejected() {
        assert_eq!(
            respond_value(
                "choice-test-unknown",
                json!({"answered": true, "selected": []}),
            )
            .unwrap_err(),
            "choice.request_not_found"
        );
    }

    #[test]
    fn cancel_session_only_closes_that_sessions_choices() {
        let first_id = "choice-test-cancel-a";
        let second_id = "choice-test-cancel-b";
        remove_pending(first_id);
        remove_pending(second_id);
        let first = register_pending(first_id, "session-cancel-a").unwrap();
        let second = register_pending(second_id, "session-cancel-b").unwrap();
        cancel_session("session-cancel-a");
        assert_eq!(
            wait(first_id, first),
            json!({"answered": false, "selected": []})
        );
        let answer = json!({"answered": true, "selected": ["option"]});
        respond_value(second_id, answer.clone()).unwrap();
        assert_eq!(wait(second_id, second), answer);
    }

    #[test]
    fn args_validation_rejects_options_out_of_bounds() {
        let one = json!({"question": "Q", "options": [{"label": "one"}]});
        assert_eq!(
            validate_args(one.as_object().unwrap()).unwrap_err(),
            "tool.invalid_args"
        );
        let seven = json!({
            "question": "Q",
            "options": (0..7).map(|index| json!({"label": index})).collect::<Vec<_>>(),
        });
        assert_eq!(
            validate_args(seven.as_object().unwrap()).unwrap_err(),
            "tool.invalid_args"
        );
    }
}
