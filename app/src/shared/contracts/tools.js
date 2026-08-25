// 内置会话源是编译期契约，不通过 Engine manifest 动态水合。
// 安装状态与扫描结果由 env/scan 查询提供，格式细节不泄漏给前端。
import { engine } from "../../platform/desktop/client.js";
import { AGENTS, AGENT_IDS } from "./generated/agents.js";

export const TOOLS = AGENT_IDS;
export const TOOL_NAME = Object.freeze(Object.fromEntries(
  TOOLS.map(tool => [tool, AGENTS[tool].displayName]),
));
export { supportsEditOperation } from "./agentEditSupport.js";

export const supportsAgentCapability = (tool, capability) =>
  Boolean(AGENTS[tool]?.capabilities?.includes(capability));

export const agentsWithCapability = capability =>
  TOOLS.filter(tool => supportsAgentCapability(tool, capability));

// Cursor 等 IDE 没有「按会话 id 接续」的 CLI；engine 的 resume 描述符
// 只会打开工作区（`cursor .`），不能真正回到那条会话。UI 应禁用接续命令
// / 终端恢复，并提示改用续聊指令或在 IDE 聊天历史里打开。
export const supportsSessionResumeCli = (tool) =>
  supportsAgentCapability(tool, "resume") && tool !== "cursor";

// 接续命令由 Engine lifecycle 生成；前端不拼装 shell 命令。
export const resumeDescriptor = (tool, ref) =>
  engine("resume", { tool, ref });
