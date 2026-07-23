// 内置会话源是编译期契约，不通过 Engine manifest 动态水合。
// 安装状态与扫描结果由 env/scan 查询提供，格式细节不泄漏给前端。
import { rpc } from "../transport/rpc.js";
export { AGENTS } from "./generated/agents.js";
import { AGENTS, AGENT_IDS } from "./generated/agents.js";

export const TOOLS = AGENT_IDS;
export const TOOL_NAME = Object.freeze(Object.fromEntries(
  TOOLS.map(tool => [tool, AGENTS[tool].displayName]),
));

const CAPABILITIES = Object.freeze([
  "browse",
  "migrate-source",
  "migrate-target",
  "edit",
  "inplace",
  "verified",
]);

export function toolManifest(tool) {
  const agent = AGENTS[tool];
  return agent ? {
    id: tool,
    display_name: agent.displayName,
    icon: agent.icon,
    capabilities: CAPABILITIES,
  } : null;
}

export function toolHasCapability(tool, capability) {
  return Boolean(AGENTS[tool]) && CAPABILITIES.includes(capability);
}

export function toolsWithCapability(capability) {
  return CAPABILITIES.includes(capability) ? TOOLS : [];
}

// 接续命令由 Engine lifecycle 生成；前端不拼装 shell 命令。
export const resumeDescriptor = (tool, sessionId, cwd) =>
  rpc("resume", { tool, session_id: sessionId, cwd: cwd || "." });
