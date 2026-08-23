//! 受控启动 OpenCode 官方 server，并调用会话编辑 API。
//!
//! 编辑路径不碰 SQLite：OpenCode 的写入必须过官方 API，否则索引/缓存会与库不一致。
//! server 用随机 basic-auth 起在 `127.0.0.1` 的随机端口上，URL 从 stdout 抓。

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use rand::RngCore as _;
use regex::Regex;
use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};
use crate::system::executables;

/// server 启动与单次请求的超时（Python `timeout=20`）。
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

static LISTENING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"opencode server listening on (https?://\S+)").expect("启动日志正则是常量")
});

/// 编辑路径需要的官方 API 子集。
pub trait OpenCodeApiClient: Send {
    /// `/doc` 里是否声明了 part 的 `patch` 路由。
    fn supports_part_patch(&self) -> DomainResult<bool>;

    /// `PATCH /session/{s}/message/{m}/part/{p}`。
    fn patch_part(&self, session_id: &str, message_id: &str, part: &Value) -> DomainResult<Value>;

    /// 会话正在运行时拒绝原地编辑。
    fn assert_idle(&self, session_id: &str) -> DomainResult<()>;
}

/// `cwd → 客户端` 的工厂；单测替换成假实现。
pub type ApiFactory = Arc<dyn Fn(&str) -> DomainResult<Box<dyn OpenCodeApiClient>> + Send + Sync>;

/// 真实实现：拉起 `opencode serve --pure` 并在 Drop 时收掉进程。
pub struct OpenCodeApi {
    cwd: String,
    timeout: Duration,
    process: Option<Child>,
    base_url: String,
    username: String,
    password: String,
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buffer);
    buffer
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl OpenCodeApi {
    /// 启动 server 并等待它报出监听地址；健康检查不通过即失败。
    pub fn start(cwd: &str, timeout: Duration) -> DomainResult<Self> {
        let username = "ferry".to_string();
        let password = random_hex(32);
        let argv = executables::argv(
            "opencode",
            &["serve", "--pure", "--hostname", "127.0.0.1", "--port", "0"],
        );
        let mut command = Command::new(&argv[0]);
        command
            .args(&argv[1..])
            .current_dir(cwd)
            .env("OPENCODE_SERVER_USERNAME", &username)
            .env("OPENCODE_SERVER_PASSWORD", &password)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process = command
            .spawn()
            .map_err(|error| DomainError::internal(format!("OpenCode server 启动失败: {error}")))?;

        // stdout 与 stderr 合流（Python 用 `stderr=STDOUT`），逐行送进队列。
        let (sender, receiver) = mpsc::channel::<String>();
        for stream in [
            process
                .stdout
                .take()
                .map(|stream| Box::new(stream) as Box<dyn std::io::Read + Send>),
            process
                .stderr
                .take()
                .map(|stream| Box::new(stream) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let sender = sender.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if sender.send(line.trim_end().to_string()).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);

        let deadline = Instant::now() + timeout;
        let mut output: Vec<String> = Vec::new();
        let mut base_url = String::new();
        while Instant::now() < deadline {
            if matches!(process.try_wait(), Ok(Some(_))) {
                break;
            }
            let Ok(line) = receiver.recv_timeout(Duration::from_millis(100)) else {
                continue;
            };
            if let Some(captures) = LISTENING_RE.captures(&line) {
                base_url = captures[1].trim_end_matches('/').to_string();
                output.push(line);
                break;
            }
            output.push(line);
        }
        let mut api = Self {
            cwd: cwd.to_string(),
            timeout,
            process: Some(process),
            base_url,
            username,
            password,
        };
        if api.base_url.is_empty() {
            api.close();
            let tail = output[output.len().saturating_sub(10)..].join("\n");
            return Err(DomainError::internal(format!(
                "OpenCode server 启动失败: {tail}"
            )));
        }
        let health = api.request("GET", "/global/health", None)?;
        if health.get("healthy") != Some(&Value::Bool(true)) {
            api.close();
            return Err(DomainError::internal("OpenCode server 健康检查失败"));
        }
        Ok(api)
    }

    fn close(&mut self) {
        let Some(mut process) = self.process.take() else {
            return;
        };
        if matches!(process.try_wait(), Ok(None)) {
            let _ = process.kill();
        }
        let _ = process.wait();
    }

    fn authorization(&self) -> String {
        let token = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", self.username, self.password));
        format!("Basic {token}")
    }

    /// 单次 HTTP 调用；空响应体返回 `Value::Null`。
    pub fn request(&self, method: &str, path: &str, body: Option<&Value>) -> DomainResult<Value> {
        let url = format!("{}{path}", self.base_url);
        // 关掉「HTTP 错误码即 Err」：Python 侧会读错误响应体拼进报错信息。
        // 用到的只有两个动词：GET 恒无请求体，PATCH 恒有。
        let authorization = self.authorization();
        let sent: Result<ureq::http::Response<ureq::Body>, ureq::Error> = match body {
            None => ureq::get(&url)
                .config()
                .timeout_global(Some(self.timeout))
                .http_status_as_error(false)
                .build()
                .header("Authorization", authorization)
                .call(),
            Some(body) => ureq::patch(&url)
                .config()
                .timeout_global(Some(self.timeout))
                .http_status_as_error(false)
                .build()
                .header("Authorization", authorization)
                .header("Content-Type", "application/json")
                .send(serde_json::to_string(body).map_err(|error| {
                    DomainError::internal(format!("请求体序列化失败: {error}"))
                })?),
        };
        let response = match sent {
            Ok(response) => response,
            Err(error) => {
                return Err(DomainError::internal(format!(
                    "OpenCode API 连接失败: {error}"
                )));
            }
        };
        let status = response.status().as_u16();
        let raw = response
            .into_body()
            .read_to_string()
            .map_err(|error| DomainError::internal(format!("OpenCode API 响应不可读: {error}")))?;
        if !(200..400).contains(&status) {
            let detail: String = {
                let characters: Vec<char> = raw.chars().collect();
                characters[characters.len().saturating_sub(500)..]
                    .iter()
                    .collect()
            };
            return Err(DomainError::internal(format!(
                "OpenCode API {method} {path} 返回 {status}: {detail}"
            )));
        }
        if raw.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&raw)
            .map_err(|error| DomainError::internal(format!("OpenCode API 响应非法: {error}")))
    }

    /// 所有会话级路由都要带 `directory` 查询参数。
    fn scoped(&self, path: &str) -> String {
        let encoded =
            percent_encoding::utf8_percent_encode(&self.cwd, percent_encoding::NON_ALPHANUMERIC)
                .to_string();
        format!("{path}?directory={encoded}")
    }
}

impl Drop for OpenCodeApi {
    fn drop(&mut self) {
        self.close();
    }
}

impl OpenCodeApiClient for OpenCodeApi {
    fn supports_part_patch(&self) -> DomainResult<bool> {
        let doc = self.request("GET", "/doc", None)?;
        let route = doc
            .get("paths")
            .and_then(Value::as_object)
            .and_then(|paths| paths.get("/session/{sessionID}/message/{messageID}/part/{partID}"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_else(Map::new);
        Ok(route.contains_key("patch"))
    }

    fn patch_part(&self, session_id: &str, message_id: &str, part: &Value) -> DomainResult<Value> {
        let part_id = part
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DomainError::internal("OpenCode part 缺少 id"))?;
        let path = format!("/session/{session_id}/message/{message_id}/part/{part_id}");
        self.request("PATCH", &self.scoped(&path), Some(part))
    }

    fn assert_idle(&self, session_id: &str) -> DomainResult<()> {
        let statuses = self.request("GET", &self.scoped("/session/status"), None)?;
        if statuses
            .as_object()
            .is_some_and(|entries| entries.contains_key(session_id))
        {
            return Err(DomainError::internal(format!(
                "OpenCode 会话 {session_id} 正在运行，拒绝原地编辑"
            )));
        }
        Ok(())
    }
}

/// 默认工厂：真起一个官方 server。
pub fn default_factory() -> ApiFactory {
    Arc::new(|cwd: &str| {
        OpenCodeApi::start(cwd, DEFAULT_TIMEOUT)
            .map(|api| Box::new(api) as Box<dyn OpenCodeApiClient>)
    })
}

static FACTORY: LazyLock<RwLock<Option<ApiFactory>>> = LazyLock::new(|| RwLock::new(None));

/// 当前生效的工厂（未安装即默认工厂）。
pub fn factory() -> ApiFactory {
    FACTORY
        .read()
        .expect("API 工厂锁中毒")
        .clone()
        .unwrap_or_else(default_factory)
}

/// 换掉工厂（单测用）。
pub fn install_factory(value: Option<ApiFactory>) {
    *FACTORY.write().expect("API 工厂锁中毒") = value;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listening_line_yields_the_base_url_without_a_trailing_slash() {
        let captures = LISTENING_RE
            .captures("INFO opencode server listening on http://127.0.0.1:53211/")
            .unwrap();
        assert_eq!(captures[1].trim_end_matches('/'), "http://127.0.0.1:53211");
        assert!(LISTENING_RE.captures("something else").is_none());
    }
}
