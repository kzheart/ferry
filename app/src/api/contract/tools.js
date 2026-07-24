// 内置会话源是编译期契约，不通过 Engine manifest 动态水合。
// 安装状态与扫描结果由 env/scan 查询提供，格式细节不泄漏给前端。
import { rpc } from "../transport/rpc.js";
export { AGENTS } from "./generated/agents.js";
import { AGENTS, AGENT_IDS } from "./generated/agents.js";

export const TOOLS = AGENT_IDS;
export const TOOL_NAME = Object.freeze(Object.fromEntries(
  TOOLS.map(tool => [tool, AGENTS[tool].displayName]),
));
export { supportsAssistantReplyEditing, supportsSessionEditing } from "./agentEditSupport.js";

// 接续命令由 Engine lifecycle 生成；前端不拼装 shell 命令。
export const resumeDescriptor = (tool, ref) =>
  rpc("resume", { tool, ref });
