//! OpenCode 当前原生结构的 import 与失败回滚。
//!
//! 写入顺序：先给整棵树分配 `ses_*` id（子会话的 task 链接要用到），再逐节点
//! `opencode import`。任何一次 import 失败都要把**已登记**的会话按逆序删掉——
//! import 可能先插入 session 再因消息 schema 失败，所以是「调用前登记」。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::errors::DomainResult;
use crate::events::Event;
use crate::model::Session;

use super::native_schema::templates;
use super::payload::{self, ToolDecider};
use super::store;

/// 写入结果：根会话的新 id + 落盘位置（`dest`）。
#[derive(Debug)]
pub struct WriteOutcome {
    pub session_id: String,
    pub dest: PathBuf,
    /// 编译期产生的损耗（Python 直接写进源 session；Rust 由调用方决定去留）。
    pub loss: Vec<Event>,
}

/// 等价 `Path(value).resolve()`：先转绝对路径，能 canonicalize 就 canonicalize，
/// 目标不存在时退回词法归一（不访问文件系统）。
fn resolved(path: &str) -> String {
    let raw = std::path::PathBuf::from(path);
    let absolute = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(raw)
    };
    std::fs::canonicalize(&absolute)
        .unwrap_or_else(|_| lexical_normalize(&absolute))
        .to_string_lossy()
        .into_owned()
}

fn lexical_normalize(path: &std::path::Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 把 canonical 会话树写进 OpenCode。
///
/// `native_payloads` 是「源就是 opencode」时的原生 payload 直通路径：不重新编译
/// 内容，只做 id 重映射，避免一次无谓的降级。
pub fn write(
    session: &Session,
    cwd: Option<&str>,
    tool_decider: Option<ToolDecider<'_>>,
    native_payloads: Option<&BTreeMap<String, Value>>,
) -> DomainResult<WriteOutcome> {
    let nodes: Vec<&Session> = session.walk();
    let mut sid_map: BTreeMap<String, String> = BTreeMap::new();
    for node in &nodes {
        sid_map.insert(node.source_id.clone(), payload::new_id("ses"));
    }
    // 父会话映射按「节点在树中的位置」建立，不能按 source_id（子树可能重名）。
    let mut parent_of: BTreeMap<usize, String> = BTreeMap::new();
    for (index, node) in nodes.iter().enumerate() {
        for child in &node.children {
            if let Some(position) = nodes
                .iter()
                .position(|candidate| std::ptr::eq(*candidate, child))
            {
                parent_of.insert(position, sid_map[&nodes[index].source_id].clone());
            }
        }
    }

    let target_cwd = resolved(cwd.unwrap_or(&session.cwd));
    let mut cached_templates: Option<Map<String, Value>> = None;
    let mut prepared: Vec<(Value, String, String)> = Vec::new();
    let mut loss: Vec<Event> = Vec::new();

    for (index, node) in nodes.iter().enumerate() {
        let sid = sid_map[&node.source_id].clone();
        let node_cwd = match cwd {
            Some(_) => target_cwd.clone(),
            None => resolved(if node.cwd.is_empty() {
                &target_cwd
            } else {
                &node.cwd
            }),
        };
        let parent_sid = parent_of.get(&index).cloned();
        let explicit = native_payloads
            .and_then(|payloads| payloads.get(&node.source_id))
            .filter(|value| value.is_object())
            .cloned();
        let has_native_payload = explicit.is_some();

        let node_payload = match explicit {
            Some(mut native) => {
                if !node.children.is_empty() {
                    let templates = cached_templates.get_or_insert_with(templates);
                    // 原生 payload 尚未重映射时，edge.spawn_message_id 仍可精确定位。
                    payload::ensure_task_links(&mut native, node, &sid, &sid_map, templates)?;
                }
                payload::remap_payload(&native, &sid, &node_cwd, parent_sid.as_deref(), &sid_map)
            }
            None => {
                let templates = cached_templates.get_or_insert_with(templates);
                let (mut compiled, node_loss) = payload::canonical_payload(
                    node,
                    &sid,
                    &node_cwd,
                    parent_sid.as_deref(),
                    templates,
                    &sid_map,
                    tool_decider,
                )?;
                loss.extend(node_loss);
                if !node.children.is_empty() {
                    payload::ensure_task_links(&mut compiled, node, &sid, &sid_map, templates)?;
                }
                compiled
            }
        };
        debug_assert!(has_native_payload || node_payload.is_object());
        prepared.push((node_payload, sid, node_cwd));
    }

    let mut imported: Vec<String> = Vec::new();
    for (node_payload, sid, node_cwd) in &prepared {
        // import 可能先插入 session 再因消息 schema 失败；调用前登记，
        // 确保半写入的当前会话也进入回滚。
        imported.push(sid.clone());
        if let Err(error) = store::import_payload(node_payload, sid, node_cwd) {
            for rolled_back in imported.iter().rev() {
                let _ = store::delete_session(rolled_back, Some(&target_cwd));
            }
            return Err(error);
        }
    }

    Ok(WriteOutcome {
        session_id: sid_map[&session.source_id].clone(),
        dest: store::database_path(),
        loss,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::shared::dialect::register_dialect;
    use crate::errors::DomainError;
    use crate::model::{
        AgentEdge, Block, BlockKind, Message, ToolCall, ToolResult, ToolResultStatus,
    };
    use crate::tool_ops::CanonicalOp;
    use serde_json::json;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeCli {
        imported: Mutex<Vec<(Value, String)>>,
        deleted: Mutex<Vec<String>>,
        fail_at: Mutex<Option<usize>>,
    }

    impl store::NativeCli for FakeCli {
        fn run_command(&self, _args: &[&str], _cwd: Option<&Path>) -> DomainResult<String> {
            Ok(String::new())
        }
        fn export_session(&self, _session_id: &str) -> DomainResult<Value> {
            Ok(Value::Null)
        }
        fn import_payload(
            &self,
            payload: &Value,
            session_id: &str,
            _cwd: &str,
        ) -> DomainResult<()> {
            let mut imported = self.imported.lock().unwrap();
            let index = imported.len();
            imported.push((payload.clone(), session_id.to_string()));
            if *self.fail_at.lock().unwrap() == Some(index) {
                return Err(DomainError::internal("child import failed"));
            }
            Ok(())
        }
        fn delete_session(&self, session_id: &str, _cwd: Option<&str>) -> DomainResult<()> {
            self.deleted.lock().unwrap().push(session_id.to_string());
            Ok(())
        }
    }

    fn install() -> (std::sync::MutexGuard<'static, ()>, Arc<FakeCli>) {
        let guard = store::tests::exclusive();
        register_dialect("opencode", &super::super::dialect::DIALECT);
        let cli = Arc::new(FakeCli::default());
        store::install_cli(cli.clone());
        (guard, cli)
    }

    fn spawn_block(call_id: &str) -> Block {
        let mut tool = ToolCall::new("Task", Some(CanonicalOp::AGENT_SPAWN.into()), json!({}));
        tool.source_call_id = Some(call_id.into());
        tool.result = Some(ToolResult::new(ToolResultStatus::Success));
        let mut block = Block::new(BlockKind::Tool);
        block.tool = Some(tool);
        block
    }

    fn tree_with_children(count: usize) -> Session {
        let mut root = Session::new("claude", "root", "/src");
        root.title = "root".into();
        let mut calls = Vec::new();
        for index in 0..count {
            let child_id = format!("child-{index}");
            let mut child = Session::new("claude", &child_id, "/src");
            child.title = child_id.clone();
            child.parent_id = Some("root".into());
            let mut message = Message::new("assistant");
            message.blocks = vec![Block::text(format!("result-{index}"))];
            message.created_at = Some(crate::model::Timestamp::Millis(200 + index as i64));
            child.messages = vec![message];
            root.children.push(child);
            let mut edge = AgentEdge::new("root", child_id);
            edge.source_call_id = Some(format!("call-{index}"));
            edge.spawn_message_id = Some("spawn".into());
            edge.prompt = format!("prompt-{index}");
            root.agent_edges.push(edge);
            calls.push(spawn_block(&format!("call-{index}")));
        }
        let mut before = Message::new("user");
        before.blocks = vec![Block::text("before")];
        before.source_id = Some("u1".into());
        before.created_at = Some(crate::model::Timestamp::Millis(100));
        let mut spawn = Message::new("assistant");
        spawn.blocks = calls;
        spawn.source_id = Some("spawn".into());
        spawn.created_at = Some(crate::model::Timestamp::Millis(200));
        let mut after = Message::new("user");
        after.blocks = vec![Block::text("after")];
        after.source_id = Some("u2".into());
        after.created_at = Some(crate::model::Timestamp::Millis(300));
        root.messages = vec![before, spawn, after];
        root
    }

    #[test]
    fn multiple_tasks_in_one_message_link_distinct_children() {
        let (_guard, cli) = install();
        let root = tree_with_children(2);
        let outcome = write(&root, Some("/src"), None, None).unwrap();
        store::reset_cli();

        let imported = cli.imported.lock().unwrap();
        let (payload, root_sid) = &imported[0];
        assert_eq!(&outcome.session_id, root_sid);
        let tasks: Vec<&Value> = payload["messages"][1]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part.get("tool") == Some(&json!("task")))
            .collect();
        assert_eq!(tasks.len(), 2);
        let call_ids: Vec<&str> = tasks
            .iter()
            .map(|task| task["callID"].as_str().unwrap())
            .collect();
        assert_eq!(call_ids, ["call-0", "call-1"]);
        let children: Vec<&str> = tasks
            .iter()
            .map(|task| task["state"]["metadata"]["sessionId"].as_str().unwrap())
            .collect();
        assert_eq!(children, [imported[1].1.as_str(), imported[2].1.as_str()]);
        assert!(tasks
            .iter()
            .all(|task| task["state"]["metadata"]["parentSessionId"] == json!(root_sid)));
        let ids: Vec<&str> = tasks
            .iter()
            .map(|task| task["id"].as_str().unwrap())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn tasks_match_children_by_call_id_not_edge_order() {
        let (_guard, cli) = install();
        let mut root = tree_with_children(2);
        root.messages[1].blocks.reverse();
        write(&root, Some("/src"), None, None).unwrap();
        store::reset_cli();

        let imported = cli.imported.lock().unwrap();
        let tasks: Vec<&Value> = imported[0].0["messages"][1]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part.get("tool") == Some(&json!("task")))
            .collect();
        let call_ids: Vec<&str> = tasks
            .iter()
            .map(|task| task["callID"].as_str().unwrap())
            .collect();
        assert_eq!(call_ids, ["call-1", "call-0"]);
        let children: Vec<&str> = tasks
            .iter()
            .map(|task| task["state"]["metadata"]["sessionId"].as_str().unwrap())
            .collect();
        assert_eq!(children, [imported[2].1.as_str(), imported[1].1.as_str()]);
    }

    #[test]
    fn a_duplicated_edge_does_not_duplicate_the_task_part() {
        let (_guard, cli) = install();
        let mut root = tree_with_children(1);
        let duplicate = root.agent_edges[0].clone();
        root.agent_edges.push(duplicate);
        write(&root, Some("/src"), None, None).unwrap();
        store::reset_cli();

        let imported = cli.imported.lock().unwrap();
        let tasks = imported[0].0["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["parts"].as_array().unwrap())
            .filter(|part| part.get("tool") == Some(&json!("task")))
            .count();
        assert_eq!(tasks, 1);
    }

    #[test]
    fn a_child_without_an_edge_gets_a_synthetic_user_and_assistant() {
        let (_guard, cli) = install();
        let mut root = Session::new("claude", "root", "/src");
        root.title = "root".into();
        let mut child = Session::new("claude", "child", "/src");
        child.parent_id = Some("root".into());
        let mut message = Message::new("assistant");
        message.blocks = vec![Block::text("result")];
        child.messages = vec![message];
        root.children = vec![child];

        write(&root, Some("/src"), None, None).unwrap();
        store::reset_cli();

        let imported = cli.imported.lock().unwrap();
        let messages = imported[0].0["messages"].as_array().unwrap();
        let roles: Vec<&str> = messages
            .iter()
            .map(|message| message["info"]["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "assistant"]);
        assert_eq!(messages[1]["info"]["parentID"], messages[0]["info"]["id"]);
        assert_eq!(messages[1]["parts"][0]["tool"], json!("task"));
    }

    #[test]
    fn explicit_session_model_fields_reach_both_message_shapes() {
        let (_guard, cli) = install();
        let mut session = Session::new("claude", "root", "/src");
        session.model_provider = Some("fixture-provider".into());
        session.model = Some("fixture-model".into());
        let mut question = Message::new("user");
        question.blocks = vec![Block::text("question")];
        let mut answer = Message::new("assistant");
        answer.blocks = vec![Block::text("answer")];
        session.messages = vec![question, answer];

        write(&session, Some("/src"), None, None).unwrap();
        store::reset_cli();

        let imported = cli.imported.lock().unwrap();
        let messages = imported[0].0["messages"].as_array().unwrap();
        assert_eq!(
            messages[0]["info"]["model"],
            json!({"providerID": "fixture-provider", "modelID": "fixture-model"})
        );
        assert_eq!(messages[1]["info"]["providerID"], json!("fixture-provider"));
        assert_eq!(messages[1]["info"]["modelID"], json!("fixture-model"));
    }

    #[test]
    fn a_second_import_failure_rolls_back_child_then_parent() {
        let (_guard, cli) = install();
        *cli.fail_at.lock().unwrap() = Some(1);
        let root = tree_with_children(1);
        let error = write(&root, Some("/src"), None, None).unwrap_err();
        store::reset_cli();

        assert_eq!(error.message(), "child import failed");
        let attempts: Vec<String> = cli
            .imported
            .lock()
            .unwrap()
            .iter()
            .map(|(_, sid)| sid.clone())
            .collect();
        let deleted = cli.deleted.lock().unwrap().clone();
        assert_eq!(deleted, attempts.iter().rev().cloned().collect::<Vec<_>>());
    }

    #[test]
    fn native_payloads_keep_their_tasks_without_adding_duplicates() {
        let (_guard, cli) = install();
        let mut root = tree_with_children(2);
        root.source_tool = "opencode".into();
        let task_parts: Vec<Value> = (0..2)
            .map(|index| {
                json!({
                    "id": format!("old-task-{index}"), "messageID": "spawn",
                    "sessionID": "root", "type": "tool", "tool": "task",
                    "callID": format!("call-{index}"),
                    "state": {"status": "completed", "input": {}, "output": "",
                              "metadata": {"parentSessionId": "root",
                                           "sessionId": format!("child-{index}")},
                              "time": {"start": 200, "end": 200}}
                })
            })
            .collect();
        let native = json!({
            "info": {"id": "root", "directory": "/src",
                     "time": {"created": 100, "updated": 300}},
            "messages": [
                {"info": {"id": "u1", "sessionID": "root", "role": "user",
                          "time": {"created": 100}},
                 "parts": [{"id": "old-u1", "type": "text", "text": "u1"}]},
                {"info": {"id": "spawn", "sessionID": "root", "role": "assistant",
                          "time": {"created": 200, "completed": 200}},
                 "parts": task_parts},
                {"info": {"id": "u2", "sessionID": "root", "role": "user",
                          "time": {"created": 300}},
                 "parts": [{"id": "old-u2", "type": "text", "text": "u2"}]},
            ]
        });
        let mut payloads = BTreeMap::new();
        payloads.insert("root".to_string(), native);

        write(&root, Some("/src"), None, Some(&payloads)).unwrap();
        store::reset_cli();

        let imported = cli.imported.lock().unwrap();
        let (payload, root_sid) = &imported[0];
        let tasks: Vec<&Value> = payload["messages"][1]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part.get("tool") == Some(&json!("task")))
            .collect();
        assert_eq!(tasks.len(), 2);
        assert!(tasks
            .iter()
            .all(|task| task["state"]["metadata"]["parentSessionId"] == json!(root_sid)));
        let children: Vec<&str> = tasks
            .iter()
            .map(|task| task["state"]["metadata"]["sessionId"].as_str().unwrap())
            .collect();
        assert_eq!(children, [imported[1].1.as_str(), imported[2].1.as_str()]);
    }

    #[test]
    fn an_empty_native_parent_can_still_link_a_child() {
        let (_guard, cli) = install();
        let mut root = Session::new("opencode", "root", "/src");
        let mut child = Session::new("opencode", "child", "/src");
        child.parent_id = Some("root".into());
        root.children = vec![child];
        let mut payloads = BTreeMap::new();
        payloads.insert(
            "root".to_string(),
            json!({"info": {"id": "root", "directory": "/src", "time": null},
                   "messages": []}),
        );
        payloads.insert(
            "child".to_string(),
            json!({"info": {"id": "child", "directory": "/src",
                            "time": {"created": 100, "updated": 100}},
                   "messages": []}),
        );

        write(&root, Some("/src"), None, Some(&payloads)).unwrap();
        store::reset_cli();

        let imported = cli.imported.lock().unwrap();
        let roles: Vec<&str> = imported[0].0["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["info"]["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "assistant"]);
        let time = &imported[0].0["info"]["time"];
        assert!(time["updated"].as_i64().unwrap() >= time["created"].as_i64().unwrap());
    }
}
