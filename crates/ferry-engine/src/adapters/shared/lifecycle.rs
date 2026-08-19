//! 格式无关的生命周期基类：文件型会话的永久删除策略。
//!
//! 语义事实源：`engine/adapters/shared/lifecycle.py`。
//!
//! Python 的 `BaseLifecycle` 是基类；Rust 用 [`BaseLifecycle`] trait 的默认方法
//! 表达同一件事，实现它即自动满足 `contracts::SessionLifecycle`（见文件末尾的
//! blanket impl），**不要**再手写一份 `impl SessionLifecycle`。
//!
//! `FileSessionLifecycle` 的删除算法需要反向拿 adapter 的 editor 与 browser，
//! WP-A 的 trait 面已经把 `&AgentAdapter` 作为 `delete` 的入参传进来，因此
//! 这里不需要额外的注入通道：实现方在自己的 `delete` 里转调
//! [`delete_file_session`] 即可。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::adapters::contracts::AgentAdapter;
use crate::errors::{DomainError, DomainResult};

use super::editing::EditDocument;

/// 通用生命周期默认值；各 Agent 实现覆盖差异点。
pub trait BaseLifecycle: Send + Sync {
    /// Agent id。
    fn tool(&self) -> &str;

    /// 装配时由 adapter 从 manifest executables 注入；缺省空串。
    fn executable(&self) -> &str {
        ""
    }

    /// `<executable> <args...>` 里的 args；必须实现。
    fn resume_args(&self, session_id: &str) -> DomainResult<Vec<String>>;

    /// 终端启动描述符：executable 必须命中 manifest 白名单。
    fn resume_descriptor(&self, session_id: &str, cwd: &str) -> DomainResult<Map<String, Value>> {
        let args = self.resume_args(session_id)?;
        let mut display = String::from("cd ");
        display.push_str(cwd);
        display.push_str(" && ");
        display.push_str(self.executable());
        for arg in &args {
            display.push(' ');
            display.push_str(arg);
        }
        let mut descriptor = Map::new();
        descriptor.insert("tool".into(), Value::from(self.tool()));
        descriptor.insert("session_id".into(), Value::from(session_id));
        descriptor.insert("cwd".into(), Value::from(cwd));
        descriptor.insert("executable".into(), Value::from(self.executable()));
        descriptor.insert(
            "args".into(),
            Value::Array(args.iter().map(|arg| Value::from(arg.as_str())).collect()),
        );
        descriptor.insert("display_command".into(), Value::from(display));
        Ok(descriptor)
    }

    /// 迁移失败时清理已写入的产物。
    fn cleanup(&self, _session_id: &str, _dest: &Path) -> DomainResult<()> {
        Err(not_implemented("cleanup"))
    }

    fn validation_ref(&self, _session_id: &str, dest: &Path) -> DomainResult<String> {
        Ok(dest.to_string_lossy().into_owned())
    }

    /// 探针是否需要工作目录；默认需要。
    fn probe_cwd(&self, cwd: Option<&str>) -> Option<String> {
        cwd.map(str::to_string)
    }

    /// 永久删除会话；默认不支持。
    fn delete(
        &self,
        _adapter: &AgentAdapter,
        _reference: &str,
    ) -> DomainResult<Map<String, Value>> {
        Err(not_implemented("delete"))
    }
}

fn not_implemented(what: &str) -> DomainError {
    DomainError::internal(format!("生命周期未实现: {what}"))
}

/// 任何 [`BaseLifecycle`] 自动满足 `contracts::SessionLifecycle`。
impl<T: BaseLifecycle> crate::adapters::contracts::SessionLifecycle for T {
    fn resume_descriptor(&self, session_id: &str, cwd: &str) -> DomainResult<Map<String, Value>> {
        BaseLifecycle::resume_descriptor(self, session_id, cwd)
    }

    fn cleanup(&self, session_id: &str, dest: &Path) -> DomainResult<()> {
        BaseLifecycle::cleanup(self, session_id, dest)
    }

    fn validation_ref(&self, session_id: &str, dest: &Path) -> DomainResult<String> {
        BaseLifecycle::validation_ref(self, session_id, dest)
    }

    fn probe_cwd(&self, cwd: Option<&str>) -> Option<String> {
        BaseLifecycle::probe_cwd(self, cwd)
    }

    fn delete(&self, adapter: &AgentAdapter, reference: &str) -> DomainResult<Map<String, Value>> {
        BaseLifecycle::delete(self, adapter, reference)
    }
}

/// 文件型会话的删除钩子：永久删除会话文件及其归属产物，不留快照。
pub trait FileSessionLifecycle: BaseLifecycle {
    /// 删除子会话，返回删除条数。
    fn delete_children(&self, _doc: &EditDocument, _path: &Path) -> DomainResult<i64> {
        Ok(0)
    }

    /// 删除随行文件（sidecar）。
    fn delete_sidecar(&self, _path: &Path) -> DomainResult<()> {
        Ok(())
    }
}

/// `FileSessionLifecycle.delete` 的算法：editor.load → 删子会话 → 删 sidecar → unlink。
///
/// Rust 的 trait 默认方法不能覆写超 trait 的默认方法，因此实现方在自己的
/// [`BaseLifecycle::delete`] 里转调本函数：
/// `fn delete(&self, adapter, reference) { delete_file_session(self, adapter, reference) }`
pub fn delete_file_session<T: FileSessionLifecycle + ?Sized>(
    lifecycle: &T,
    adapter: &AgentAdapter,
    reference: &str,
) -> DomainResult<Map<String, Value>> {
    let editor = adapter.require_editor()?;
    let doc = editor.load(reference)?;
    let path = match doc.handle.downcast_ref::<PathBuf>() {
        Some(path) => path.clone(),
        None => PathBuf::from(adapter.require_browser()?.resolve_ref(reference)?),
    };
    let children = lifecycle.delete_children(&doc, &path)?;
    lifecycle.delete_sidecar(&path)?;
    fs::remove_file(&path).map_err(|error| {
        DomainError::internal(format!("删除会话文件失败: {}: {error}", path.display()))
    })?;
    let mut result = Map::new();
    result.insert("ok".into(), Value::Bool(true));
    result.insert("children".into(), Value::from(children));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Probe;

    impl BaseLifecycle for Probe {
        fn tool(&self) -> &str {
            "claude"
        }

        fn executable(&self) -> &str {
            "/usr/local/bin/claude"
        }

        fn resume_args(&self, session_id: &str) -> DomainResult<Vec<String>> {
            Ok(vec!["--resume".into(), session_id.to_string()])
        }
    }

    #[test]
    fn resume_descriptor_shape_matches_python() {
        let descriptor = Probe.resume_descriptor("abc", "/work/dir").unwrap();
        let keys: Vec<&str> = descriptor.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "tool",
                "session_id",
                "cwd",
                "executable",
                "args",
                "display_command"
            ]
        );
        assert_eq!(
            descriptor["display_command"],
            Value::from("cd /work/dir && /usr/local/bin/claude --resume abc")
        );
        assert_eq!(descriptor["args"], serde_json::json!(["--resume", "abc"]));
    }

    #[test]
    fn probe_cwd_passes_the_directory_through() {
        assert_eq!(Probe.probe_cwd(Some("/a")), Some("/a".to_string()));
        assert_eq!(Probe.probe_cwd(None), None);
    }

    #[test]
    fn validation_ref_defaults_to_the_destination_path() {
        assert_eq!(
            Probe
                .validation_ref("abc", Path::new("/tmp/x.jsonl"))
                .unwrap(),
            "/tmp/x.jsonl"
        );
    }

    #[test]
    fn unimplemented_hooks_report_internal_errors() {
        let error = Probe.cleanup("abc", Path::new("/tmp")).unwrap_err();
        assert_eq!(error.code, "internal.unexpected");
    }
}
