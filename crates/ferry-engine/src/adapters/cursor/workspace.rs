//! 目标工作区解析：把 Ferry 的 `cwd` 对上 Cursor 的工作区哈希。
//!
//! Cursor 的会话列表按 `composerHeaders.workspaceId` 分桶：这个哈希对不上，迁入的
//! 会话就存在于库里却不出现在用户打开的那个文件夹里。哈希由 Cursor 自己算（VS Code
//! 的 workspace hash），Ferry 不复算，只**查**它已经落过的两处事实：
//!
//! 1. 同一个文件夹已经有别的会话 → 直接复用那行的 `workspaceId`（最强证据）；
//! 2. `User/workspaceStorage/<hash>/workspace.json` 的 `folder` 指向该文件夹。
//!
//! 两处都查不到，说明这个文件夹从没在 Cursor 里打开过——此时**拒绝写入**而不是
//! 编一个哈希：编出来的会话在 Cursor 里永远不会显示，是一次静默失败。

use std::path::{Path, PathBuf};

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};

use super::store;

/// 路径转 URI 时保留的字符：unreserved 集合再加分隔符 `/`。
const PATH_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// 一个已被 Cursor 认识的目标工作区。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workspace {
    /// Cursor 的工作区哈希（= `workspaceStorage/<hash>/` 目录名）。
    pub id: String,
    /// 工作区文件夹的绝对路径。
    pub path: String,
}

impl Workspace {
    /// `file://` URI，`conversationState` 的 f9 与 head 的 `external` 都用它。
    pub fn uri(&self) -> String {
        file_uri(&self.path)
    }

    /// VS Code 序列化的 URI 对象。
    pub fn uri_value(&self) -> Value {
        let mut uri = Map::new();
        uri.insert("$mid".into(), Value::from(1));
        uri.insert("fsPath".into(), Value::from(self.path.as_str()));
        uri.insert("external".into(), Value::from(self.uri()));
        uri.insert("path".into(), Value::from(self.path.as_str()));
        uri.insert("scheme".into(), Value::from("file"));
        Value::Object(uri)
    }

    /// head / composerData 里的 `workspaceIdentifier`。
    pub fn identifier(&self) -> Value {
        let mut identifier = Map::new();
        identifier.insert("id".into(), Value::from(self.id.as_str()));
        identifier.insert("uri".into(), self.uri_value());
        Value::Object(identifier)
    }
}

/// 绝对路径 → `file://` URI。
pub fn file_uri(path: &str) -> String {
    format!("file://{}", utf8_percent_encode(path, PATH_SET))
}

/// `file://` URI → 绝对路径；非 file scheme 返回 `None`。
pub fn uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///path` 的 authority 为空；带 authority 的远程 URI 不是本地工作区。
    if !rest.starts_with('/') {
        return None;
    }
    percent_encoding::percent_decode_str(rest)
        .decode_utf8()
        .ok()
        .map(|decoded| decoded.into_owned())
}

fn same_folder(left: &str, right: &str) -> bool {
    let normalize = |value: &str| value.trim_end_matches('/').to_string();
    !left.is_empty() && normalize(left) == normalize(right)
}

/// 从 head JSON 里取工作区路径。
fn head_workspace_path(head: &Value) -> Option<&str> {
    let uri = head.get("workspaceIdentifier")?.get("uri")?;
    for key in ["fsPath", "path"] {
        if let Some(value) = uri.get(key).and_then(Value::as_str) {
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// 已有会话里同一文件夹用过的工作区哈希；取最近使用的那行。
fn from_existing_sessions(connection: &Connection, cwd: &str) -> Option<String> {
    let mut statement = connection
        .prepare(
            "SELECT workspaceId, value FROM composerHeaders \
             WHERE workspaceId IS NOT NULL AND workspaceId <> '' \
             ORDER BY COALESCE(recency, lastUpdatedAt, createdAt) DESC",
        )
        .ok()?;
    let mut rows = statement.query([]).ok()?;
    while let Some(row) = rows.next().ok()? {
        let identifier = store::text_cell(row.get_ref(0).ok()?);
        let head: Value = serde_json::from_str(&store::text_cell(row.get_ref(1).ok()?)).ok()?;
        if head_workspace_path(&head).is_some_and(|path| same_folder(path, cwd)) {
            return Some(identifier);
        }
    }
    None
}

/// `<globalStorage>/../workspaceStorage`。
fn workspace_storage_root() -> Option<PathBuf> {
    Some(
        store::database_path()
            .parent()?
            .parent()?
            .join("workspaceStorage"),
    )
}

/// 扫 `workspaceStorage/<hash>/workspace.json` 找 `folder` 指向该文件夹的那一个。
fn from_workspace_storage(root: &Path, cwd: &str) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path().join("workspace.json")) else {
            continue;
        };
        let Ok(document) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let matched = document
            .get("folder")
            .and_then(Value::as_str)
            .and_then(uri_path)
            .is_some_and(|path| same_folder(&path, cwd));
        if matched {
            candidates.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    // 同一个文件夹可能有多份历史目录；取字典序最小的那个，保证结果稳定。
    candidates.sort();
    candidates.into_iter().next()
}

/// 解析目标工作区；Cursor 没见过这个文件夹时报错而不是编一个哈希。
pub fn resolve(connection: &Connection, cwd: &str) -> DomainResult<Workspace> {
    let identifier = from_existing_sessions(connection, cwd)
        .or_else(|| workspace_storage_root().and_then(|root| from_workspace_storage(&root, cwd)));
    match identifier {
        Some(id) => Ok(Workspace {
            id,
            path: cwd.to_string(),
        }),
        None => Err(DomainError::session_store_unavailable(
            "cursor",
            &format!("Cursor 没有这个工作区的记录: {cwd}；请先在 Cursor 里打开该文件夹一次再迁入"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::cursor::store::tests::{exclusive, materialize};
    use serde_json::json;

    #[test]
    fn file_uris_round_trip_through_percent_encoding() {
        assert_eq!(file_uri("/Users/u/work"), "file:///Users/u/work");
        assert_eq!(
            file_uri("/Users/u/我的 项目"),
            "file:///Users/u/%E6%88%91%E7%9A%84%20%E9%A1%B9%E7%9B%AE"
        );
        assert_eq!(
            uri_path("file:///Users/u/%E6%88%91%E7%9A%84%20%E9%A1%B9%E7%9B%AE").as_deref(),
            Some("/Users/u/我的 项目")
        );
        // 非本地 scheme 与远程 authority 都不是工作区。
        assert_eq!(uri_path("vscode-remote://host/w"), None);
        assert_eq!(uri_path("file://host/w"), None);
    }

    #[test]
    fn an_existing_session_in_the_same_folder_donates_its_hash() {
        let _guard = exclusive();
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("state.vscdb");
        materialize(
            &database,
            &json!({"sessions": [{"id": "s1", "header": {
                "createdAt": 100,
                "workspaceIdentifier": {"id": "3d6aae0c", "uri": {
                    "$mid": 1, "scheme": "file", "fsPath": "/w", "path": "/w",
                    "external": "file:///w"}}}}]}),
        );
        store::set_database_path_override(Some(database.clone()));
        let connection = store::open_readonly(&database).unwrap();
        let workspace = resolve(&connection, "/w").unwrap();
        assert_eq!(workspace.id, "3d6aae0c");
        assert_eq!(workspace.uri(), "file:///w");
        assert_eq!(workspace.identifier()["uri"]["fsPath"], json!("/w"));
        // 尾斜杠不该让同一个文件夹认不出来。
        assert_eq!(resolve(&connection, "/w/").unwrap().id, "3d6aae0c");
        // 没见过的文件夹拒绝写入。
        let error = resolve(&connection, "/other").unwrap_err();
        assert_eq!(error.code, "session.store_unavailable");
        assert!(error.message().contains("请先在 Cursor 里打开该文件夹"));
        store::set_database_path_override(None);
    }

    #[test]
    fn workspace_storage_supplies_the_hash_when_no_session_exists_yet() {
        let root = tempfile::tempdir().unwrap();
        let storage = root.path().join("workspaceStorage");
        for (name, folder) in [
            ("aaa111", "file:///w/other"),
            ("bbb222", "file:///w/target"),
        ] {
            let directory = storage.join(name);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join("workspace.json"),
                json!({"folder": folder}).to_string(),
            )
            .unwrap();
        }
        assert_eq!(
            from_workspace_storage(&storage, "/w/target").as_deref(),
            Some("bbb222")
        );
        assert_eq!(from_workspace_storage(&storage, "/w/missing"), None);
    }
}
