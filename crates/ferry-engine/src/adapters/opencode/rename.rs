//! OpenCode 标题写回。
//!
//! 走官方 HTTP API `PATCH /session/{id}`（`session.update`，请求体 `{"title"}`），由
//! server 负责写 `session.title`、递增 `event_sequence` 并发 `session.updated`。
//! 直写 `opencode.db` 会绕过 durable event stream，并与正在跑的会话抢 WAL 写锁，
//! 所以不做。标题不是 `New session - <ISO>` 占位格式后，OpenCode 的 ensureTitle 不会
//! 再自动覆盖它。
//!
//! 已知限制：Ferry 起的是自己的临时 server，用户正在运行的 OpenCode TUI/桌面端是另一个
//! 进程，收不到这次 `session.updated`，要重启才能看到新标题。

use serde_json::{Map, Value};

use crate::adapters::contracts::SessionRenamer;
use crate::errors::{DomainError, DomainResult};

use super::api::{self, ApiFactory, OpenCodeApiClient};
use super::store;

pub const RESTART_NOTE: &str = "OpenCode 正在运行时需重启后才会显示新标题";

pub struct OpenCodeRenamer {
    api_factory: ApiFactory,
}

impl Default for OpenCodeRenamer {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenCodeRenamer {
    pub fn new() -> Self {
        Self {
            api_factory: api::factory(),
        }
    }

    pub fn with_factory(api_factory: ApiFactory) -> Self {
        Self { api_factory }
    }
}

/// 用一个已就绪的客户端改标题；与进程/库无关，便于单测。
pub fn rename_with(
    client: &dyn OpenCodeApiClient,
    session_id: &str,
    title: &str,
) -> DomainResult<Map<String, Value>> {
    if !client.supports_session_update()? {
        return Err(DomainError::internal(
            "当前 OpenCode server 不支持官方 session 更新 API",
        ));
    }
    client.assert_idle(session_id)?;
    let updated = client.set_title(session_id, title)?;
    let saved_title = updated
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(title)
        .to_string();

    let mut result = Map::new();
    result.insert("title".into(), Value::from(saved_title));
    result.insert("session_id".into(), Value::from(session_id));
    result.insert(
        "saved_as".into(),
        Value::from(store::database_path().to_string_lossy().into_owned()),
    );
    result.insert("via".into(), Value::from("session.update"));
    result.insert(
        "notes".into(),
        Value::Array(vec![Value::from(RESTART_NOTE)]),
    );
    Ok(result)
}

impl SessionRenamer for OpenCodeRenamer {
    fn rename(&self, reference: &str, title: &str) -> DomainResult<Map<String, Value>> {
        let payload = store::load_native_payload(reference)?;
        let cwd = payload
            .get("info")
            .and_then(|info| info.get("directory"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(".")
            .to_string();
        let client = (self.api_factory)(&cwd)?;
        rename_with(client.as_ref(), reference, title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::opencode::editor::testing::{recorder, Client};
    use serde_json::json;

    #[test]
    fn rename_patches_the_session_title_through_the_api() {
        let (state, _factory) = recorder();
        let client = Client(state.clone());
        let result = rename_with(&client, "ses_1", "New Title").unwrap();
        assert_eq!(result["title"], json!("New Title"));
        assert_eq!(result["via"], json!("session.update"));
        assert_eq!(
            state.titles.lock().unwrap().as_slice(),
            &[("ses_1".to_string(), "New Title".to_string())]
        );
    }

    #[test]
    fn rename_refuses_a_running_session_or_an_old_server() {
        let (state, _factory) = recorder();
        *state.busy.lock().unwrap() = true;
        let client = Client(state.clone());
        assert!(rename_with(&client, "ses_1", "x").is_err());
        assert!(state.titles.lock().unwrap().is_empty());

        *state.busy.lock().unwrap() = false;
        *state.supports.lock().unwrap() = false;
        assert!(rename_with(&client, "ses_1", "x").is_err());
        assert!(state.titles.lock().unwrap().is_empty());
    }
}
