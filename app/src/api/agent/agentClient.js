// Ask Ferry 传输层:经 Tauri `agent_command` 走 ferry-agent/v1 协议,
// 审批走独立可信命令(approve 与 apply 在 Rust 内一次完成,凭证不进 WebView)
import { invoke } from "@tauri-apps/api/core";

const PROTOCOL = "ferry-agent/v1";
let requestSeq = 1;

export class AgentError extends Error {
  constructor(code, message) {
    super(message || code);
    this.code = code;
  }
}

export async function agentCommand(method, params) {
  const request = JSON.stringify({
    protocol: PROTOCOL,
    id: `ui_${Date.now().toString(36)}_${requestSeq++}`,
    method,
    params: params || {},
  });
  let raw;
  try {
    raw = await invoke("agent_command", { request });
  } catch (error) {
    throw new AgentError("agent_unavailable", String(error));
  }
  const response = JSON.parse(raw);
  if (!response.ok) {
    throw new AgentError(response.error?.code || "agent_error", response.error?.message);
  }
  return response.result;
}

// 事件流:runtime 事件与 Rust 补发的 operation.proposed / runtime.disconnected 共用同一通道
export async function onAgentEvent(handler) {
  const { listen } = await import("@tauri-apps/api/event");
  return listen("ferry-agent-event", e => handler(e.payload));
}

export const operationPlanApply = planId =>
  invoke("operation_apply", { planId }).then(raw => {
    const response = JSON.parse(raw);
    if (!response.ok) throw new AgentError(
      response.error?.code || "operation_apply_failed",
      response.error?.message,
    );
    return response.result;
  });
