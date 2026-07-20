// Agent 清单单一事实源:引擎 tools RPC 下发,应用启动时水合(见 main.jsx)。
// TOOLS/TOOL_NAME 原地更新,保证既有引用在水合后拿到最新清单。
import { rpc } from "../transport/rpc.js";

let manifests = [];
export const TOOLS = [];
export const TOOL_NAME = {};

export function hydrateTools(list) {
  manifests = Array.isArray(list) ? list : [];
  TOOLS.length = 0;
  for (const key of Object.keys(TOOL_NAME)) delete TOOL_NAME[key];
  for (const m of manifests) {
    TOOLS.push(m.id);
    TOOL_NAME[m.id] = m.display_name || m.id;
  }
}

export const toolManifests = () => manifests;

export async function loadTools() {
  try { hydrateTools(await rpc("tools")); } catch { /* 引擎不可用时保持空清单 */ }
}

// 接续命令由引擎 lifecycle 生成(launch descriptor),前端不再拼装
export const resumeDescriptor = (tool, sessionId, cwd) =>
  rpc("resume", { tool, session_id: sessionId, cwd: cwd || "." });
