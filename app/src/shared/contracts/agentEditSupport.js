import { AGENT_IDS } from "./generated/agents.js";

// 当前原生结构的编辑范围是编译期契约，不通过运行时 capability RPC 探测。
// OpenCode 的当前官方写入路径只能安全改写消息，不能替换整轮助手回复。
export const supportsSessionEditing = tool => AGENT_IDS.includes(tool);
export const supportsAssistantReplyEditing = tool =>
  tool === "claude" || tool === "codex";
