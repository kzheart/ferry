//! 迁移用例：预览、写入后的结构校验、可选影子探针、回滚与历史审计。
//!
//! 硬约束（§2.4 第 25 条）：
//! `write → validate_written_tree（re-read 验收）→ 结构失败才回滚（探针失败不回滚）
//!  → history.append（回滚也写）`；executor 侧还有第二道门禁。
//!
//! 与 Python 的一处结构差异（注释里点明）：`session` 由调用方传入并按值持有
//! （planner / executor 一定会传），因此不再需要 `sessions.read_tree` 回落路径。
//!
//! `narration.content_locale` 是**线程局部**作用域：写入与预览都必须包在守卫
//! 里（Python 的 `with narration.content_locale(content_locale)`）。哪怕当前
//! RPC 面恒传 `None` 也不能省——守卫的作用之一就是把上一次操作可能遗留在同一
//! 条 worker 线程上的 locale 清掉。

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::contracts::MigrationTarget;
use crate::adapters::shared::narration;
use crate::model::Session;
use crate::operations::types::{EngineError, EngineResult, Ports};
use crate::operations::{history, verification};
use crate::storage::database::now_ms;

/// 目标 adapter `write()` 返回的两个必需键。
const WRITE_SESSION_ID: &str = "session_id";
const WRITE_DEST: &str = "dest";

pub struct MigrationService {
    ports: Ports,
}

/// `_prepare` 的产物。
struct Prepared {
    session: Session,
    target_cwd: String,
    base: Map<String, Value>,
}

impl MigrationService {
    pub fn new(ports: Ports) -> Self {
        Self { ports }
    }

    pub fn resume_command(
        &self,
        tool: &str,
        session_id: &str,
        cwd: &str,
    ) -> EngineResult<Map<String, Value>> {
        let adapter = self.ports.adapter(tool)?;
        let lifecycle = adapter.require_lifecycle("resume")?;
        Ok(lifecycle.resume_descriptor(session_id, cwd)?)
    }

    /// 只读预览。`content_locale` 目前恒为 `None`（RPC 面不暴露），守卫仍要装。
    #[allow(clippy::too_many_arguments)]
    pub fn preview(
        &self,
        src: &str,
        dst: &str,
        session: Session,
        cwd: Option<&str>,
        max_turn: Option<i64>,
        probe_model: Option<&str>,
        content_locale: Option<&str>,
    ) -> EngineResult<Map<String, Value>> {
        let prepared = self.prepare(src, dst, session, cwd, max_turn, probe_model)?;
        let target_adapter = self.ports.adapter(dst)?;
        let target = target_adapter.require_migration_target()?;
        let preview = {
            let _locale = narration::content_locale(content_locale, None);
            target.preview(&prepared.session, Some(&prepared.target_cwd))?
        };
        let mut result = prepared.base;
        result.insert("preview".into(), Value::Object(preview));
        Ok(result)
    }

    /// 写入 + 验收 + 可选探针 + 回滚 + 历史。
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &self,
        src: &str,
        dst: &str,
        session: Session,
        cwd: Option<&str>,
        probe: bool,
        max_turn: Option<i64>,
        probe_model: Option<&str>,
        content_locale: Option<&str>,
    ) -> EngineResult<Map<String, Value>> {
        let target_adapter = self.ports.adapter(dst)?;
        target_adapter.require_migration_target()?;
        target_adapter.require_browser()?;
        target_adapter.require_lifecycle("resume")?;
        if probe {
            target_adapter.require_verifier("probe")?;
        }
        let prepared = self.prepare(src, dst, session, cwd, max_turn, probe_model)?;
        let target = target_adapter.require_migration_target()?;
        let (session_id, destination) = {
            let _locale = narration::content_locale(content_locale, None);
            write_artifact(target, &prepared.session, &prepared.target_cwd)?
        };

        // 产物是否还在：结构失败回滚后会置 false，异常路径就不再重复清理。
        let mut artifact_active = true;
        match self.apply_after_write(
            dst,
            &prepared,
            target,
            &session_id,
            &destination,
            probe,
            probe_model,
            content_locale,
            &mut artifact_active,
        ) {
            Ok(result) => Ok(result),
            Err(error) => {
                if artifact_active {
                    let _ = self.cleanup_artifact(dst, &session_id, &destination);
                }
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_after_write(
        &self,
        dst: &str,
        prepared: &Prepared,
        target: &dyn MigrationTarget,
        session_id: &str,
        destination: &Path,
        probe: bool,
        probe_model: Option<&str>,
        content_locale: Option<&str>,
        artifact_active: &mut bool,
    ) -> EngineResult<Map<String, Value>> {
        let mut result = prepared.base.clone();
        result.insert(
            "loss".into(),
            Value::Object(target.plan(&prepared.session)?),
        );
        result.insert(WRITE_SESSION_ID.into(), Value::from(session_id));
        result.insert(
            WRITE_DEST.into(),
            Value::from(destination.to_string_lossy().into_owned()),
        );
        result.insert(
            "resume".into(),
            Value::Object(self.resume_command(dst, session_id, &prepared.target_cwd)?),
        );

        let expected_shape = tree_shape(&prepared.session);
        let (ok, tree_detail) =
            self.validate_written_tree(dst, session_id, destination, &expected_shape);

        let mut structure = Map::new();
        structure.insert("ok".into(), Value::Bool(ok));
        structure.insert("detail".into(), Value::from(tree_detail.as_str()));
        let mut runtime = Map::new();
        runtime.insert("status".into(), Value::from("skipped"));
        let mut validation = Map::new();
        validation.insert("structure".into(), Value::Object(structure));
        validation.insert("runtime".into(), Value::Object(runtime));

        let mut runtime_report: Option<Map<String, Value>> = None;
        if ok && probe {
            let report = self.isolated_probe(
                dst,
                &prepared.session,
                &prepared.target_cwd,
                probe_model,
                content_locale,
            )?;
            let mut with_model = report.clone();
            with_model.insert("model".into(), optional_string(probe_model));
            validation.insert("runtime".into(), Value::Object(with_model));
            runtime_report = Some(report);
        }
        result.insert("validation".into(), Value::Object(validation));

        // 探针失败不回滚：产物已过结构验收，失败多源于目标环境而非迁移本身。
        if probe || !ok {
            let mut report = match runtime_report {
                Some(report) => report,
                None => structure_probe_report(ok, &tree_detail),
            };
            if probe {
                report.insert("model".into(), optional_string(probe_model));
            }
            result.insert("probe".into(), Value::Object(report));
        }

        if !ok {
            self.cleanup_artifact(dst, session_id, destination)?;
            *artifact_active = false;
            result.insert("rolled_back".into(), Value::Bool(true));
        }

        // 回滚也写历史：审计要能看到「试过并且失败了」。
        let mut entry = result.clone();
        entry.insert("time".into(), Value::from(now_ms()));
        history::append(&Value::Object(entry), &self.ports)?;
        Ok(result)
    }

    fn prepare(
        &self,
        src: &str,
        dst: &str,
        mut session: Session,
        cwd: Option<&str>,
        max_turn: Option<i64>,
        probe_model: Option<&str>,
    ) -> EngineResult<Prepared> {
        let source_adapter = self.ports.adapter(src)?;
        source_adapter.require_migration_source()?;
        if let Some(max_turn) = max_turn.filter(|turn| *turn != 0) {
            truncate_rounds(&mut session, max_turn);
        }
        let target_adapter = self.ports.adapter(dst)?;
        let target = target_adapter.require_migration_target()?;
        let target_cwd = resolve_cwd(cwd, &session.cwd);
        let stats = target.plan(&session)?;
        let tree_count = session.walk().len() as i64;
        let message_count = session.message_count() as i64;
        let edge_count: i64 = session
            .walk()
            .iter()
            .map(|node| node.agent_edges.len() as i64)
            .sum();

        let mut topology = Map::new();
        topology.insert("nodes".into(), Value::from(tree_count));
        topology.insert("edges".into(), Value::from(0.max(tree_count - 1)));
        topology.insert("agent_edges".into(), Value::from(edge_count));
        topology.insert("preserved".into(), Value::Bool(true));
        topology.insert(
            "detail".into(),
            Value::from(if tree_count > 1 {
                "父子会话关系将按原拓扑写入"
            } else {
                "普通单会话,无子会话拓扑"
            }),
        );

        let mut base = Map::new();
        base.insert("src".into(), Value::from(src));
        base.insert("dst".into(), Value::from(dst));
        base.insert("source_id".into(), Value::from(session.source_id.as_str()));
        base.insert("title".into(), Value::from(session.title.as_str()));
        base.insert("cwd".into(), Value::from(target_cwd.as_str()));
        base.insert("loss".into(), Value::Object(stats));
        base.insert("tree_count".into(), Value::from(tree_count));
        base.insert("child_count".into(), Value::from(tree_count - 1));
        base.insert("topology".into(), Value::Object(topology));
        base.insert(
            "max_turn".into(),
            match max_turn {
                Some(turn) => Value::from(turn),
                None => Value::Null,
            },
        );
        base.insert("msg_count".into(), Value::from(message_count));
        base.insert(
            "root_msg_count".into(),
            Value::from(session.messages.len() as i64),
        );
        base.insert("probe_model".into(), optional_string(probe_model));

        Ok(Prepared {
            session,
            target_cwd,
            base,
        })
    }

    /// 影子副本探针：写一份一次性产物、跑探针、无论成败都清理掉。
    ///
    /// Python 版还会保存/还原每个节点的 `loss`（`target.write` 会往里追加），
    /// Rust 的 `MigrationTarget::write` 只拿 `&Session`，天然不会污染，故省略。
    fn isolated_probe(
        &self,
        dst: &str,
        session: &Session,
        cwd: &str,
        model: Option<&str>,
        content_locale: Option<&str>,
    ) -> EngineResult<Map<String, Value>> {
        // Python 侧整个 `_isolated_probe` 调用都在 `with content_locale(...)` 里。
        let _locale = narration::content_locale(content_locale, None);
        let target_adapter = self.ports.adapter(dst)?;
        let target = target_adapter.require_migration_target()?;
        let (shadow_session_id, shadow_destination) = write_artifact(target, session, cwd)?;
        let outcome = self.run_probe(dst, &shadow_session_id, cwd, model);
        let cleanup = self.cleanup_artifact(dst, &shadow_session_id, &shadow_destination);
        let mut report = outcome?;
        cleanup?;
        if !report.contains_key("isolation") {
            let mut isolation = Map::new();
            isolation.insert("kind".into(), Value::from("shadow_copy"));
            isolation.insert("id".into(), Value::from(shadow_session_id.as_str()));
            isolation.insert("cleaned".into(), Value::Bool(true));
            report.insert("isolation".into(), Value::Object(isolation));
        }
        Ok(report)
    }

    /// 探针超时不上抛，折成 `probe.timeout` 报告。
    pub fn run_probe(
        &self,
        tool: &str,
        session_id: &str,
        cwd: &str,
        model: Option<&str>,
    ) -> EngineResult<Map<String, Value>> {
        let adapter = self.ports.adapter(tool)?;
        let lifecycle = adapter.require_lifecycle("resume")?;
        let probe_cwd = lifecycle.probe_cwd(Some(cwd));
        match verification::run_probe(tool, session_id, probe_cwd.as_deref(), model, &self.ports) {
            Ok(report) => Ok(verification::report_to_value(&report)
                .as_object()
                .cloned()
                .unwrap_or_default()),
            Err(error) if verification::is_probe_timeout(&error) => {
                Ok(verification::timeout_report(tool, error.message())
                    .as_object()
                    .cloned()
                    .unwrap_or_default())
            }
            Err(error) => Err(error),
        }
    }

    /// re-read 验收：节点数 / id 去重数 / 父子边数 / 层级拓扑四项全中才算通过。
    pub fn validate_written_tree(
        &self,
        tool: &str,
        session_id: &str,
        destination: &Path,
        expected_shape: &TreeShape,
    ) -> (bool, String) {
        match self.read_written_tree(tool, session_id, destination) {
            Ok(restored) => {
                let nodes = restored.walk();
                let mut ids: Vec<&str> = nodes.iter().map(|node| node.source_id.as_str()).collect();
                let node_count = nodes.len() as i64;
                let edge_count: i64 = nodes.iter().map(|node| node.children.len() as i64).sum();
                ids.sort_unstable();
                ids.dedup();
                let expected = 1 + count_shape_nodes(expected_shape);
                let shape_matches = &tree_shape(&restored) == expected_shape;
                let ok = node_count == expected
                    && ids.len() as i64 == expected
                    && edge_count == 0.max(expected - 1)
                    && shape_matches;
                let detail = format!(
                    "树结构验收: 节点 {node_count}/{expected}, 父子边 {edge_count}/{}, 层级拓扑 {}",
                    0.max(expected - 1),
                    if shape_matches { "一致" } else { "不一致" }
                );
                (ok, detail)
            }
            Err(error) => (false, format!("树结构验收失败: {}", error.message())),
        }
    }

    fn read_written_tree(
        &self,
        tool: &str,
        session_id: &str,
        destination: &Path,
    ) -> EngineResult<Session> {
        let adapter = self.ports.adapter(tool)?;
        let browser = adapter.require_browser()?;
        let lifecycle = adapter.require_lifecycle("resume")?;
        let reference = lifecycle.validation_ref(session_id, destination)?;
        Ok(browser.read(&reference)?)
    }

    fn cleanup_artifact(
        &self,
        dst: &str,
        session_id: &str,
        destination: &Path,
    ) -> EngineResult<()> {
        let adapter = self.ports.adapter(dst)?;
        let lifecycle = adapter.require_lifecycle("resume")?;
        lifecycle.cleanup(session_id, destination)?;
        Ok(())
    }
}

fn write_artifact(
    target: &dyn MigrationTarget,
    session: &Session,
    cwd: &str,
) -> EngineResult<(String, PathBuf)> {
    let written = target.write(session, cwd)?;
    let session_id = written
        .get(WRITE_SESSION_ID)
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::key_error(WRITE_SESSION_ID))?
        .to_string();
    let destination = written
        .get(WRITE_DEST)
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::key_error(WRITE_DEST))?;
    Ok((session_id, PathBuf::from(destination)))
}

fn optional_string(value: Option<&str>) -> Value {
    match value.filter(|text| !text.is_empty()) {
        Some(text) => Value::from(text),
        None => Value::Null,
    }
}

/// 结构验收失败时的兜底 probe 报告。
fn structure_probe_report(ok: bool, detail: &str) -> Map<String, Value> {
    let mut diagnostic = Map::new();
    diagnostic.insert("stdout".into(), Value::from(detail));
    diagnostic.insert("stderr".into(), Value::from(""));
    diagnostic.insert("truncated".into(), Value::Bool(false));
    let mut report = Map::new();
    report.insert(
        "status".into(),
        Value::from(if ok { "passed" } else { "failed" }),
    );
    report.insert(
        "code".into(),
        if ok {
            Value::Null
        } else {
            Value::from("probe.structure_invalid")
        },
    );
    report.insert("params".into(), Value::Object(Map::new()));
    report.insert("diagnostic".into(), Value::Object(diagnostic));
    report
}

/// `Path(cwd or source.cwd or ".").resolve()`。
///
/// Python 的 `Path.resolve()` 对不存在的路径也返回绝对路径，所以这里不能用
/// `canonicalize`（要求存在），改成「已是绝对路径就原样，否则拼当前目录」。
fn resolve_cwd(cwd: Option<&str>, session_cwd: &str) -> String {
    let raw = cwd
        .filter(|value| !value.is_empty())
        .or(Some(session_cwd).filter(|value| !value.is_empty()))
        .unwrap_or(".");
    let path = crate::system::paths::expanduser(raw);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    normalize(&absolute).to_string_lossy().into_owned()
}

fn normalize(path: &Path) -> PathBuf {
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

/// 层级拓扑：每个节点折成「子节点形状的有序列表」。
///
/// Python 用 `tuple(sorted(..., key=repr))`，排序键是 tuple 的 repr 字符串；
/// [`shape_repr`] 逐字复刻那个 repr，保证排序结果一致。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeShape(pub Vec<TreeShape>);

pub fn tree_shape(session: &Session) -> TreeShape {
    let mut shapes: Vec<TreeShape> = session.children.iter().map(tree_shape).collect();
    shapes.sort_by_key(shape_repr);
    TreeShape(shapes)
}

/// 逐字复刻 Python 嵌套 tuple 的 `repr()`，排序键必须和它一致。
fn shape_repr(shape: &TreeShape) -> String {
    match shape.0.len() {
        0 => "()".to_string(),
        1 => format!("({},)", shape_repr(&shape.0[0])),
        _ => {
            let inner: Vec<String> = shape.0.iter().map(shape_repr).collect();
            format!("({})", inner.join(", "))
        }
    }
}

fn count_shape_nodes(shape: &TreeShape) -> i64 {
    shape
        .0
        .iter()
        .map(|child| 1 + count_shape_nodes(child))
        .sum()
}

/// 按用户轮次截断，并同步剪掉失去 spawn 锚点的子会话与边。
pub fn truncate_rounds(session: &mut Session, max_turn: i64) {
    let mut kept = Vec::new();
    let mut turn = 0_i64;
    for message in &session.messages {
        if message.role == "user" {
            turn += 1;
        }
        if turn > max_turn {
            break;
        }
        kept.push(message.clone());
    }
    let dropped = session.messages.len() - kept.len();
    if dropped > 0 {
        let mut params = Map::new();
        params.insert("max_turn".into(), Value::from(max_turn));
        params.insert("dropped".into(), Value::from(dropped as i64));
        session.lose("migration.truncated", params);
    }
    session.messages = kept;

    let kept_ids: Vec<&str> = session
        .messages
        .iter()
        .filter_map(|message| message.source_id.as_deref())
        .collect();
    let child_ids: Vec<String> = session
        .children
        .iter()
        .map(|child| child.source_id.clone())
        .collect();
    let mut edges = Vec::new();
    let mut kept_children: Vec<String> = Vec::new();
    for edge in &session.agent_edges {
        let anchored = edge
            .spawn_message_id
            .as_deref()
            .is_some_and(|id| kept_ids.contains(&id));
        if !child_ids.contains(&edge.child_session_id)
            || !anchored
            || kept_children.contains(&edge.child_session_id)
        {
            continue;
        }
        edges.push(edge.clone());
        kept_children.push(edge.child_session_id.clone());
    }
    let original_children = session.children.len();
    session
        .children
        .retain(|child| kept_children.contains(&child.source_id));
    let removed = original_children - session.children.len();
    if removed > 0 {
        let mut params = Map::new();
        params.insert("count".into(), Value::from(removed as i64));
        session.lose("migration.children_not_migrated", params);
    }
    session.agent_edges = edges;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentEdge, Message};

    fn session(source_id: &str) -> Session {
        Session::new("claude", source_id, "/tmp")
    }

    fn message(role: &str, source_id: &str) -> Message {
        Message {
            source_id: Some(source_id.to_string()),
            ..Message::new(role)
        }
    }

    #[test]
    fn shape_repr_matches_python_tuple_repr() {
        let leaf = TreeShape(Vec::new());
        assert_eq!(shape_repr(&leaf), "()");
        assert_eq!(shape_repr(&TreeShape(vec![leaf.clone()])), "((),)");
        assert_eq!(
            shape_repr(&TreeShape(vec![leaf.clone(), leaf.clone()])),
            "((), ())"
        );
        assert_eq!(
            shape_repr(&TreeShape(vec![TreeShape(vec![leaf])])),
            "(((),),)"
        );
    }

    #[test]
    fn tree_shape_is_order_independent() {
        let mut left = session("root");
        left.children = vec![session("a"), {
            let mut child = session("b");
            child.children = vec![session("c")];
            child
        }];
        let mut right = session("root");
        right.children = vec![
            {
                let mut child = session("b");
                child.children = vec![session("c")];
                child
            },
            session("a"),
        ];
        assert_eq!(tree_shape(&left), tree_shape(&right));
        assert_eq!(count_shape_nodes(&tree_shape(&left)), 3);
    }

    #[test]
    fn truncate_rounds_drops_messages_and_orphaned_children() {
        let mut value = session("root");
        value.messages = vec![
            message("user", "m1"),
            message("assistant", "m2"),
            message("user", "m3"),
            message("assistant", "m4"),
        ];
        value.children = vec![session("child-1"), session("child-2")];
        let mut first = AgentEdge::new("root", "child-1");
        first.spawn_message_id = Some("m1".into());
        let mut second = AgentEdge::new("root", "child-2");
        second.spawn_message_id = Some("m3".into());
        value.agent_edges = vec![first, second];

        truncate_rounds(&mut value, 1);

        assert_eq!(value.messages.len(), 2);
        assert_eq!(
            value
                .children
                .iter()
                .map(|child| child.source_id.as_str())
                .collect::<Vec<_>>(),
            ["child-1"]
        );
        assert_eq!(value.agent_edges.len(), 1);
        let codes: Vec<&str> = value.loss.iter().map(|event| event.code.as_str()).collect();
        assert_eq!(
            codes,
            ["migration.truncated", "migration.children_not_migrated"]
        );
    }

    #[test]
    fn structure_failure_report_carries_the_ghost_free_code() {
        let report = structure_probe_report(false, "树结构验收: 节点 1/2");
        assert_eq!(report["status"], Value::from("failed"));
        assert_eq!(report["code"], Value::from("probe.structure_invalid"));
        assert_eq!(
            report["diagnostic"]["stdout"],
            Value::from("树结构验收: 节点 1/2")
        );

        let passed = structure_probe_report(true, "ok");
        assert_eq!(passed["status"], Value::from("passed"));
        assert_eq!(passed["code"], Value::Null);
    }
}
