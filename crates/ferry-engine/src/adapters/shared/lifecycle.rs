//! 格式无关的生命周期基类：文件型会话的永久删除策略。
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
        let mut descriptor = descriptor_of(self.tool(), cwd, self.executable(), &args);
        descriptor.insert("session_id".into(), Value::from(session_id));
        // 键序与 Python 一致：tool / session_id / cwd / executable / args / display_command。
        reorder_descriptor(descriptor)
    }

    /// 迁移失败时清理已写入的产物。
    fn cleanup(&self, _session_id: &str, _dest: &Path) -> DomainResult<()> {
        Err(not_implemented("cleanup"))
    }

    fn validation_ref(&self, _session_id: &str, dest: &Path) -> DomainResult<String> {
        Ok(dest.to_string_lossy().into_owned())
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

/// POSIX shell 的最小引用：安全字符集之外的一律套单引号，单引号自身写成 `'\''`。
///
/// 交接引导语里带空格与中文标点，`display_command` 是给用户直接粘进终端的，
/// 不引用就会被 shell 拆成多个参数。安全字符集内的参数原样输出，因此既有
/// `resume` 描述符（`--resume abc`、`.`）的文本一个字节都不变。
pub fn shell_quote(argument: &str) -> String {
    const SAFE: &str = "@%+=:,./-_";
    if !argument.is_empty()
        && argument
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || SAFE.contains(character))
    {
        return argument.to_string();
    }
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('\'');
    for character in argument.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }
    quoted.push('\'');
    quoted
}

/// 描述符公共字段：`display_command` 里的 cwd 与每个参数都过 [`shell_quote`]。
fn descriptor_of(tool: &str, cwd: &str, executable: &str, args: &[String]) -> Map<String, Value> {
    let mut display = String::from("cd ");
    display.push_str(&shell_quote(cwd));
    display.push_str(" && ");
    display.push_str(&shell_quote(executable));
    for argument in args {
        display.push(' ');
        display.push_str(&shell_quote(argument));
    }
    let mut descriptor = Map::new();
    descriptor.insert("tool".into(), Value::from(tool));
    descriptor.insert("cwd".into(), Value::from(cwd));
    descriptor.insert("executable".into(), Value::from(executable));
    descriptor.insert(
        "args".into(),
        Value::Array(args.iter().map(|arg| Value::from(arg.as_str())).collect()),
    );
    descriptor.insert("display_command".into(), Value::from(display));
    descriptor
}

/// resume 描述符的键序是既有契约的一部分，插入 `session_id` 后重排回去。
fn reorder_descriptor(mut descriptor: Map<String, Value>) -> DomainResult<Map<String, Value>> {
    let mut ordered = Map::new();
    for key in [
        "tool",
        "session_id",
        "cwd",
        "executable",
        "args",
        "display_command",
    ] {
        if let Some(value) = descriptor.remove(key) {
            ordered.insert(key.into(), value);
        }
    }
    Ok(ordered)
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

    /// 安全字符集内的参数一个字节都不引：既有 resume 描述符的文本因此不变。
    #[test]
    fn shell_quote_only_wraps_what_the_shell_would_split() {
        for plain in [
            "--resume",
            ".",
            "abc",
            "/work/dir",
            "a_b-c.d+e:f,g@h%i=j",
            "01a02803-9a5f-7b91",
        ] {
            assert_eq!(shell_quote(plain), plain, "{plain} 不该被引起来");
        }
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("/Users/a b/项目"), "'/Users/a b/项目'");
        assert_eq!(
            shell_quote("请先读取 /tmp/hf_x.md 并按其中说明接手。"),
            "'请先读取 /tmp/hf_x.md 并按其中说明接手。'"
        );
        // 单引号自身：闭合 → 转义的单引号 → 重新开引号。
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
        assert_eq!(shell_quote("a\nb"), "'a\nb'");
    }

    #[test]
    fn a_descriptor_quotes_the_cwd_and_every_argument() {
        struct Spaced;
        impl BaseLifecycle for Spaced {
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
        // cwd 与参数都可能带空格，描述符必须逐项引号包裹后再拼 display_command。
        let descriptor = Spaced
            .resume_descriptor("s 1", "/work dir")
            .expect("resume 描述符");
        assert_eq!(
            descriptor["display_command"],
            Value::from("cd '/work dir' && /usr/local/bin/claude --resume 's 1'")
        );
    }
}
