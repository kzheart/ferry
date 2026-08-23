//! Operation agent prompt 入口；具体 CLI 执行由 adapter verifier 持有。

use serde_json::{Map, Value};

use crate::operations::types::{EngineResult, Ports};

/// 等价 `verification.run_agent_prompt`；`timeout` 默认 360 秒由分发层给。
pub fn run_agent_prompt(
    tool: &str,
    session_id: &str,
    prompt: &str,
    dirpath: Option<&str>,
    model: Option<&str>,
    timeout: u64,
    ports: &Ports,
) -> EngineResult<Map<String, Value>> {
    let adapter = ports.adapter(tool)?;
    let verifier = adapter.require_verifier("prompt")?;
    Ok(verifier.prompt_session(session_id, dirpath, prompt, model, timeout)?)
}
