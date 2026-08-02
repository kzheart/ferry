//! ask_user 的宿主侧挂起、应答与会话生命周期管理。

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;
use tauri::AppHandle;

use crate::contracts::ipc::FERRY_IPC_PROTOCOL;

use super::emit_host_event;

const CHOICE_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_CUSTOM_TEXT_CHARS: usize = 2000;
/// 终态记录只用于挡住「终态事件先于注册到达」的毫秒级窗口,留最近若干条即可。
const MAX_FINISHED_RUNS: usize = 64;

struct PendingChoice {
    session_id: String,
    labels: HashSet<String>,
    multi_select: bool,
    allow_custom: bool,
    sender: Sender<Value>,
}

#[derive(Default)]
struct ChoiceState {
    pending: HashMap<String, PendingChoice>,
    finished: HashSet<(String, String)>,
    finished_order: VecDeque<(String, String)>,
}

impl ChoiceState {
    fn finish(&mut self, session_id: &str, run_id: &str) {
        let key = (session_id.to_owned(), run_id.to_owned());
        if !self.finished.insert(key.clone()) {
            return;
        }
        self.finished_order.push_back(key);
        while self.finished_order.len() > MAX_FINISHED_RUNS {
            if let Some(evicted) = self.finished_order.pop_front() {
                self.finished.remove(&evicted);
            }
        }
    }
}

static STATE: OnceLock<Mutex<ChoiceState>> = OnceLock::new();

/// 中毒的锁会让选择通道永久不可用,临界区不含可 panic 的代码,直接取回内部值。
fn state() -> MutexGuard<'static, ChoiceState> {
    STATE
        .get_or_init(|| Mutex::new(ChoiceState::default()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn fallback_answer() -> Value {
    json!({"answered": false, "selected": []})
}

fn invalid_args() -> String {
    "tool.invalid_args".to_owned()
}

fn invalid_answer() -> String {
    "choice.invalid_answer".to_owned()
}

#[derive(Debug)]
struct ChoiceRequest {
    question: String,
    options: Value,
    labels: HashSet<String>,
    multi_select: bool,
    allow_custom: bool,
}

fn validate_args(args: &Map<String, Value>) -> Result<ChoiceRequest, String> {
    let question = args
        .get("question")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.chars().count() <= 500)
        .ok_or_else(invalid_args)?
        .to_owned();
    let raw_options = args
        .get("options")
        .and_then(Value::as_array)
        .filter(|options| (2..=6).contains(&options.len()))
        .ok_or_else(invalid_args)?;
    let mut labels = HashSet::new();
    let mut options = Vec::with_capacity(raw_options.len());
    for option in raw_options {
        let object = option.as_object().ok_or_else(invalid_args)?;
        let label = object
            .get("label")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.chars().count() <= 80)
            .ok_or_else(invalid_args)?;
        if !labels.insert(label.to_owned()) {
            return Err(invalid_args());
        }
        // 只保留契约声明的三个字段,未知字段不进 UI 事件。
        let mut rebuilt = Map::new();
        rebuilt.insert("label".to_owned(), Value::String(label.to_owned()));
        if let Some(description) = object.get("description") {
            let description = description
                .as_str()
                .filter(|text| text.chars().count() <= 200)
                .ok_or_else(invalid_args)?;
            rebuilt.insert(
                "description".to_owned(),
                Value::String(description.to_owned()),
            );
        }
        if let Some(recommended) = object.get("recommended") {
            if !recommended.is_boolean() {
                return Err(invalid_args());
            }
            rebuilt.insert("recommended".to_owned(), recommended.clone());
        }
        options.push(Value::Object(rebuilt));
    }
    let multi_select = match args.get("multi_select") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(invalid_args)?,
    };
    let allow_custom = match args.get("allow_custom") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(invalid_args)?,
    };
    Ok(ChoiceRequest {
        question,
        options: Value::Array(options),
        labels,
        multi_select,
        allow_custom,
    })
}

/// 应答必须落在当初问出去的那张卡的取值范围内,否则等于让 UI 自由投喂模型。
fn validate_answer(entry: &PendingChoice, answer: &Value) -> Result<(), String> {
    let object = answer.as_object().ok_or_else(invalid_answer)?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "answered" | "selected" | "custom_text"))
        || object.get("answered").and_then(Value::as_bool).is_none()
    {
        return Err(invalid_answer());
    }
    let selected = object
        .get("selected")
        .and_then(Value::as_array)
        .ok_or_else(invalid_answer)?;
    if !entry.multi_select && selected.len() > 1 {
        return Err(invalid_answer());
    }
    for item in selected {
        let label = item.as_str().ok_or_else(invalid_answer)?;
        if !entry.labels.contains(label) {
            return Err(invalid_answer());
        }
    }
    if let Some(custom_text) = object.get("custom_text") {
        let custom_text = custom_text.as_str().ok_or_else(invalid_answer)?;
        if !entry.allow_custom || custom_text.chars().count() > MAX_CUSTOM_TEXT_CHARS {
            return Err(invalid_answer());
        }
    }
    Ok(())
}

enum Registration {
    Waiting(Receiver<Value>),
    RunFinished,
}

fn register_pending(
    request_id: &str,
    session_id: &str,
    run_id: &str,
    request: &ChoiceRequest,
) -> Result<Registration, String> {
    let (sender, receiver) = mpsc::channel();
    let mut guard = state();
    // 终态事件可能在 tool.request 的工作线程注册之前就被读到,那时挂起表还是空的,
    // 注册完就没人来唤醒了。
    if guard
        .finished
        .contains(&(session_id.to_owned(), run_id.to_owned()))
    {
        return Ok(Registration::RunFinished);
    }
    if guard.pending.contains_key(request_id) {
        return Err("choice.request_id_in_use".to_owned());
    }
    guard.pending.insert(
        request_id.to_owned(),
        PendingChoice {
            session_id: session_id.to_owned(),
            labels: request.labels.clone(),
            multi_select: request.multi_select,
            allow_custom: request.allow_custom,
            sender,
        },
    );
    Ok(Registration::Waiting(receiver))
}

fn remove_pending(request_id: &str) {
    state().pending.remove(request_id);
}

fn respond_value(session_id: &str, request_id: &str, answer: Value) -> Result<(), String> {
    let sender = {
        let mut guard = state();
        let entry = guard
            .pending
            .get(request_id)
            .filter(|entry| entry.session_id == session_id)
            .ok_or_else(|| "choice.request_not_found".to_owned())?;
        // 校验通过才摘走挂起项:一次非法应答不该把卡片打死。
        validate_answer(entry, &answer)?;
        guard
            .pending
            .remove(request_id)
            .ok_or_else(|| "choice.request_not_found".to_owned())?
            .sender
    };
    sender
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

fn answer_payload(request_id: &str, tool_call_id: &str, answer: &Value) -> Value {
    let mut payload = Map::new();
    payload.insert(
        "request_id".to_owned(),
        Value::String(request_id.to_owned()),
    );
    payload.insert(
        "tool_call_id".to_owned(),
        Value::String(tool_call_id.to_owned()),
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
    if session_id.is_empty() || request_id.is_empty() {
        return Err(invalid_args());
    }
    let request = validate_args(args)?;
    let Registration::Waiting(receiver) =
        register_pending(request_id, session_id, run_id, &request)?
    else {
        // run 已经终态,不再向 UI 弹卡,直接把默认答案还给模型。
        return Ok(fallback_answer());
    };
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
                "question": request.question,
                "options": request.options,
                "multi_select": request.multi_select,
                "allow_custom": request.allow_custom,
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
            "payload": answer_payload(request_id, tool_call_id, &answer),
        }),
    );
    Ok(answer)
}

#[tauri::command]
pub(crate) fn choice_respond(
    session_id: String,
    request_id: String,
    answer: Value,
) -> Result<(), String> {
    respond_value(&session_id, &request_id, answer)
}

/// run 走到终态:唤醒该会话所有挂起的选择,并记住这个 run 已经结束。
pub(super) fn finish_run(session_id: &str, run_id: &str) {
    let entries = {
        let mut guard = state();
        guard.finish(session_id, run_id);
        let request_ids: Vec<String> = guard
            .pending
            .iter()
            .filter(|(_, entry)| entry.session_id == session_id)
            .map(|(request_id, _)| request_id.clone())
            .collect();
        request_ids
            .into_iter()
            .filter_map(|request_id| guard.pending.remove(&request_id))
            .collect::<Vec<_>>()
    };
    for entry in entries {
        let _ = entry.sender.send(fallback_answer());
    }
}

/// Runtime 进程退出:不会再有终态事件,挂起的选择必须就地了结,否则线程要空等到超时。
pub(super) fn cancel_all() {
    let entries = {
        let mut guard = state();
        guard.finished.clear();
        guard.finished_order.clear();
        guard
            .pending
            .drain()
            .map(|(_, entry)| entry)
            .collect::<Vec<_>>()
    };
    for entry in entries {
        let _ = entry.sender.send(fallback_answer());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// 挂起表是进程级单例,cancel_all 会波及并行跑的其他用例,这里把选择用例串行化。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reset() {
        let mut guard = state();
        guard.pending.clear();
        guard.finished.clear();
        guard.finished_order.clear();
    }

    fn request(multi_select: bool, allow_custom: bool) -> ChoiceRequest {
        let args = json!({
            "question": "Q",
            "options": [{"label": "keep"}, {"label": "delete"}],
            "multi_select": multi_select,
            "allow_custom": allow_custom,
        });
        validate_args(args.as_object().unwrap()).unwrap()
    }

    fn entry_for(request: &ChoiceRequest) -> PendingChoice {
        PendingChoice {
            session_id: "session".to_owned(),
            labels: request.labels.clone(),
            multi_select: request.multi_select,
            allow_custom: request.allow_custom,
            sender: mpsc::channel().0,
        }
    }

    fn register(request_id: &str, session_id: &str) -> Receiver<Value> {
        match register_pending(request_id, session_id, "run", &request(false, false)).unwrap() {
            Registration::Waiting(receiver) => receiver,
            Registration::RunFinished => panic!("run 仍在进行中"),
        }
    }

    fn wait(request_id: &str, receiver: Receiver<Value>) -> Value {
        wait_for_answer(request_id, receiver)
    }

    #[test]
    fn respond_wakes_waiter_and_returns_original_value() {
        let _serial = serial();
        reset();
        let request_id = "choice-test-response";
        let receiver = register(request_id, "session-response");
        let waiter = thread::spawn(move || wait(request_id, receiver));
        let answer = json!({"answered": true, "selected": ["keep"]});
        respond_value("session-response", request_id, answer.clone()).unwrap();
        assert_eq!(waiter.join().unwrap(), answer);
    }

    #[test]
    fn unknown_request_id_is_rejected() {
        let _serial = serial();
        reset();
        assert_eq!(
            respond_value(
                "session-unknown",
                "choice-test-unknown",
                json!({"answered": true, "selected": []}),
            )
            .unwrap_err(),
            "choice.request_not_found"
        );
    }

    #[test]
    fn answers_from_another_session_are_rejected() {
        let _serial = serial();
        reset();
        let request_id = "choice-test-cross-session";
        let receiver = register(request_id, "session-owner");
        assert_eq!(
            respond_value(
                "session-intruder",
                request_id,
                json!({"answered": true, "selected": ["keep"]}),
            )
            .unwrap_err(),
            "choice.request_not_found"
        );
        // 被拒的应答不能顺手把挂起项吃掉。
        let answer = json!({"answered": true, "selected": ["keep"]});
        respond_value("session-owner", request_id, answer.clone()).unwrap();
        assert_eq!(wait(request_id, receiver), answer);
    }

    #[test]
    fn answers_must_stay_inside_the_offered_choice() {
        let _serial = serial();
        let single = entry_for(&request(false, false));
        let multi = entry_for(&request(true, true));
        for answer in [
            json!({"answered": true, "selected": ["never-offered"]}),
            json!({"answered": true, "selected": ["keep", "delete"]}),
            json!({"answered": true, "selected": [], "custom_text": "写点别的"}),
            json!({"answered": true, "selected": [1]}),
            json!({"answered": true, "selected": [], "extra": true}),
            json!({"selected": []}),
        ] {
            assert_eq!(
                validate_answer(&single, &answer).unwrap_err(),
                "choice.invalid_answer",
                "应当拒绝: {answer}"
            );
        }
        let overlong = json!({
            "answered": true,
            "selected": [],
            "custom_text": "字".repeat(MAX_CUSTOM_TEXT_CHARS + 1),
        });
        assert_eq!(
            validate_answer(&multi, &overlong).unwrap_err(),
            "choice.invalid_answer"
        );

        validate_answer(&single, &json!({"answered": true, "selected": ["keep"]})).unwrap();
        validate_answer(
            &multi,
            &json!({"answered": true, "selected": ["keep", "delete"], "custom_text": "补充"}),
        )
        .unwrap();
    }

    #[test]
    fn finish_run_only_closes_that_sessions_choices() {
        let _serial = serial();
        reset();
        let first_id = "choice-test-cancel-a";
        let second_id = "choice-test-cancel-b";
        let first = register(first_id, "session-cancel-a");
        let second = register(second_id, "session-cancel-b");
        finish_run("session-cancel-a", "run");
        assert_eq!(
            wait(first_id, first),
            json!({"answered": false, "selected": []})
        );
        let answer = json!({"answered": true, "selected": ["keep"]});
        respond_value("session-cancel-b", second_id, answer.clone()).unwrap();
        assert_eq!(wait(second_id, second), answer);
    }

    #[test]
    fn registering_after_the_run_finished_falls_back_immediately() {
        let _serial = serial();
        reset();
        finish_run("session-finished", "run-finished");
        assert!(matches!(
            register_pending(
                "choice-test-late",
                "session-finished",
                "run-finished",
                &request(false, false),
            )
            .unwrap(),
            Registration::RunFinished
        ));
        // 同一会话的下一个 run 不受影响。
        assert!(matches!(
            register_pending(
                "choice-test-late",
                "session-finished",
                "run-next",
                &request(false, false),
            )
            .unwrap(),
            Registration::Waiting(_)
        ));
    }

    #[test]
    fn finished_runs_do_not_grow_without_bound() {
        let _serial = serial();
        reset();
        for index in 0..(MAX_FINISHED_RUNS * 2) {
            finish_run("session-bounded", &format!("run-{index}"));
        }
        let guard = state();
        assert_eq!(guard.finished.len(), MAX_FINISHED_RUNS);
        assert_eq!(guard.finished_order.len(), MAX_FINISHED_RUNS);
    }

    #[test]
    fn cancel_all_closes_every_pending_choice() {
        let _serial = serial();
        reset();
        let first_id = "choice-test-all-a";
        let second_id = "choice-test-all-b";
        let first = register(first_id, "session-all-a");
        let second = register(second_id, "session-all-b");
        cancel_all();
        assert_eq!(
            wait(first_id, first),
            json!({"answered": false, "selected": []})
        );
        assert_eq!(
            wait(second_id, second),
            json!({"answered": false, "selected": []})
        );
        assert!(state().pending.is_empty());
    }

    #[test]
    fn args_validation_rejects_options_out_of_bounds() {
        let _serial = serial();
        let one = json!({"question": "Q", "options": [{"label": "one"}]});
        assert_eq!(
            validate_args(one.as_object().unwrap()).unwrap_err(),
            "tool.invalid_args"
        );
        let seven = json!({
            "question": "Q",
            "options": (0..7).map(|index| json!({"label": format!("option-{index}")}))
                .collect::<Vec<_>>(),
        });
        assert_eq!(
            validate_args(seven.as_object().unwrap()).unwrap_err(),
            "tool.invalid_args"
        );
    }

    #[test]
    fn unknown_option_fields_are_dropped_before_reaching_the_ui() {
        let _serial = serial();
        let args = json!({
            "question": "Q",
            "options": [
                {"label": "keep", "description": "保留", "recommended": true, "onclick": "x"},
                {"label": "delete"},
            ],
        });
        let request = validate_args(args.as_object().unwrap()).unwrap();
        assert_eq!(
            request.options,
            json!([
                {"label": "keep", "description": "保留", "recommended": true},
                {"label": "delete"},
            ])
        );
    }
}
