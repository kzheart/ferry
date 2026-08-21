//! 实例仲裁：`~/.ferry/engine.lock` 与陈旧 socket 清理。
//!
//! 全系统单引擎实例是这一波的核心不变量：App sidecar 与 CLI daemon 共用
//! `ferry-state.sqlite3` / `content-index.sqlite3`，双写场景必须在绑定这一步
//! 就被挡掉。锁文件是 JSON `{pid, mode, socket, version, contract_hash}`，
//! 正常退出时删除；被 kill -9 之类打断时留下陈旧锁，靠下一次启动的 pid 活性
//! 校验兜底——所以锁文件本身不是真相，**pid 活性 + socket 可连**才是。

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::contracts::ipc::FERRY_CONTRACT_HASH;
use crate::server::socket::platform;
use crate::server::socket::EngineMode;
use crate::system::snapshots::data_dir;

/// socket 路径：`--socket` 显式值 > `FERRY_ENGINE_SOCKET` > `~/.ferry/engine.sock`。
pub fn default_socket_path() -> PathBuf {
    match std::env::var("FERRY_ENGINE_SOCKET") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => data_dir().join("engine.sock"),
    }
}

/// 锁文件与 socket 同处 `~/.ferry`（`FERRY_DATA_DIR` 可整体改道，测试据此隔离）。
pub fn lock_path() -> PathBuf {
    data_dir().join("engine.lock")
}

/// daemon 自拉起时的日志落点。
pub fn daemon_log_path() -> PathBuf {
    data_dir().join("daemon.log")
}

/// 锁文件内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockRecord {
    pub pid: u32,
    pub mode: EngineMode,
    pub socket: String,
    pub version: String,
    pub contract_hash: String,
}

impl LockRecord {
    pub fn current(mode: EngineMode, socket: &Path) -> Self {
        Self {
            pid: std::process::id(),
            mode,
            socket: socket.display().to_string(),
            version: crate::context::ENGINE_VERSION.to_string(),
            contract_hash: FERRY_CONTRACT_HASH.to_string(),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("pid".into(), Value::from(self.pid));
        payload.insert("mode".into(), Value::from(self.mode.as_str()));
        payload.insert("socket".into(), Value::from(self.socket.as_str()));
        payload.insert("version".into(), Value::from(self.version.as_str()));
        payload.insert(
            "contract_hash".into(),
            Value::from(self.contract_hash.as_str()),
        );
        Value::Object(payload)
    }

    fn from_json(value: &Value) -> Option<Self> {
        let pid = value.get("pid").and_then(Value::as_u64)?;
        Some(Self {
            pid: u32::try_from(pid).ok()?,
            mode: EngineMode::parse(
                value
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )?,
            socket: value.get("socket").and_then(Value::as_str)?.to_string(),
            version: value
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            contract_hash: value
                .get("contract_hash")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }
}

/// 读锁；文件缺失、非法 JSON、字段缺失一律当「没有锁」。
pub fn read(path: &Path) -> Option<LockRecord> {
    let text = std::fs::read_to_string(path).ok()?;
    LockRecord::from_json(&serde_json::from_str::<Value>(&text).ok()?)
}

pub fn write(path: &Path, record: &LockRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 {}: {error}", parent.display()))?;
    }
    std::fs::write(path, record.to_json().to_string())
        .map_err(|error| format!("无法写入锁文件 {}: {error}", path.display()))?;
    set_owner_only(path);
    Ok(())
}

#[cfg(unix)]
fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) {}

/// 当前占用者。`None` 表示可以绑定（陈旧残留由 [`bind_exclusive`] 清理）。
pub fn occupant(socket: &Path, lock: &Path) -> Option<LockRecord> {
    let record = read(lock);
    if let Some(record) = &record {
        if record.pid != std::process::id() && platform::process_alive(record.pid) {
            return Some(record.clone());
        }
    }
    // 锁丢了但确实有人在听：连得上就说明活着，不能清。
    if socket.exists() && platform::connect(socket).is_ok() {
        return Some(record.unwrap_or(LockRecord {
            pid: 0,
            mode: EngineMode::App,
            socket: socket.display().to_string(),
            version: String::new(),
            contract_hash: String::new(),
        }));
    }
    None
}

/// 绑定期间持有的清理责任：socket 文件与锁文件都随它一起消失。
pub struct Binding {
    socket: PathBuf,
    lock: PathBuf,
}

impl Binding {
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// 显式清理（优雅退出路径）；`Drop` 会再做一次，重复删除无害。
    pub fn release(&self) {
        let _ = std::fs::remove_file(&self.socket);
        // 只删自己的锁：别人的锁在并发抢占时可能已经覆盖上来了。
        if read(&self.lock).is_some_and(|record| record.pid == std::process::id()) {
            let _ = std::fs::remove_file(&self.lock);
        }
    }
}

impl Drop for Binding {
    fn drop(&mut self) {
        self.release();
    }
}

/// 仲裁 + 绑定：陈旧残留清掉，活实例直接拒绝。
pub fn bind_exclusive(
    socket: &Path,
    lock: &Path,
    mode: EngineMode,
) -> Result<(Binding, platform::SocketListener), String> {
    if let Some(holder) = occupant(socket, lock) {
        return Err(format!(
            "已有 Ferry 引擎实例在使用 {}（pid={} mode={}）；\
             App 模式请退出 Ferry App，daemon 模式请先 `ferry daemon stop`",
            socket.display(),
            holder.pid,
            holder.mode.as_str()
        ));
    }
    // 到这里剩下的都是陈旧残留：持有者已死或从未存在。
    if socket.exists() {
        std::fs::remove_file(socket)
            .map_err(|error| format!("无法清理陈旧 socket {}: {error}", socket.display()))?;
    }
    let _ = std::fs::remove_file(lock);
    let listener = platform::bind(socket)?;
    let binding = Binding {
        socket: socket.to_path_buf(),
        lock: lock.to_path_buf(),
    };
    write(lock, &LockRecord::current(mode, socket))?;
    Ok((binding, listener))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_record_round_trips() {
        let record = LockRecord {
            pid: 4242,
            mode: EngineMode::Daemon,
            socket: "/tmp/engine.sock".into(),
            version: "0.1.0".into(),
            contract_hash: "abc".into(),
        };
        let parsed = LockRecord::from_json(&record.to_json()).expect("锁记录可解析");
        assert_eq!(parsed, record);
    }

    #[test]
    fn malformed_locks_are_treated_as_absent() {
        let dir = tempfile::tempdir().expect("临时目录可创建");
        let path = dir.path().join("engine.lock");
        assert!(read(&path).is_none());
        std::fs::write(&path, "{").expect("可写");
        assert!(read(&path).is_none());
        std::fs::write(&path, r#"{"pid": 1}"#).expect("可写");
        assert!(read(&path).is_none(), "字段不全的锁不能当真");
    }

    #[cfg(unix)]
    #[test]
    fn stale_lock_is_cleaned_and_live_lock_is_refused() {
        let dir = tempfile::tempdir().expect("临时目录可创建");
        let socket = dir.path().join("engine.sock");
        let lock = dir.path().join("engine.lock");

        // 陈旧：锁指向一个不存在的 pid，socket 文件是上次崩溃留下的空壳。
        std::fs::write(&socket, b"stale").expect("可写");
        write(
            &lock,
            &LockRecord {
                pid: dead_pid(),
                mode: EngineMode::Daemon,
                socket: socket.display().to_string(),
                version: "0.1.0".into(),
                contract_hash: "abc".into(),
            },
        )
        .expect("锁可写");
        let (binding, _listener) =
            bind_exclusive(&socket, &lock, EngineMode::Daemon).expect("陈旧残留应被清理");
        assert_eq!(
            read(&lock).expect("锁已重写").pid,
            std::process::id(),
            "绑定成功必须改写锁的持有者"
        );

        // 活着：同一个 socket 上再绑一次必须被拒。
        let error = match bind_exclusive(&socket, &lock, EngineMode::Daemon) {
            Ok(_) => panic!("活实例占用时不该绑定成功"),
            Err(error) => error,
        };
        assert!(error.contains("已有 Ferry 引擎实例"), "{error}");

        binding.release();
        assert!(!socket.exists());
        assert!(!lock.exists());
    }

    /// 一个几乎不可能存活的 pid：先探测，探到活的就往上找。
    #[cfg(unix)]
    fn dead_pid() -> u32 {
        (1..1000)
            .map(|offset| 900_000 - offset)
            .find(|pid| !platform::process_alive(*pid))
            .expect("总能找到一个不存在的 pid")
    }
}
