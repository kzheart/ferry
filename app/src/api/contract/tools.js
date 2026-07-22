// Agent 清单单一事实源:引擎 tools RPC 下发,应用启动时水合(见 main.jsx)。
// TOOLS/TOOL_NAME 原地更新,保证既有引用在水合后拿到最新清单。
import { rpc } from "../transport/rpc.js";

const CACHE_KEY = "ferry-tools-manifests";
let manifests = [];
export const TOOLS = [];
export const TOOL_NAME = {};
const listeners = new Set();

// 清单实际变化时通知订阅方(App 用它把"全选"态筛选器扩展到新全集)
export function onToolsHydrated(cb) {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

export function hydrateTools(list) {
  const next = Array.isArray(list) ? list : [];
  const changed = JSON.stringify(next) !== JSON.stringify(manifests);
  manifests = next;
  TOOLS.length = 0;
  for (const key of Object.keys(TOOL_NAME)) delete TOOL_NAME[key];
  for (const m of manifests) {
    TOOLS.push(m.id);
    TOOL_NAME[m.id] = m.display_name || m.id;
  }
  if (changed) listeners.forEach(cb => cb([...TOOLS]));
}

// 秒开:启动先用上次缓存的清单同步水合,引擎就绪后 loadTools 再校准
export function hydrateToolsFromCache() {
  try {
    const cached = JSON.parse(localStorage.getItem(CACHE_KEY) || "null");
    if (Array.isArray(cached) && cached.length) hydrateTools(cached);
  } catch { /* 缓存损坏则忽略,等引擎清单 */ }
}

export const toolManifests = () => manifests;

export function toolManifest(tool) {
  return manifests.find(item => item.id === tool) || null;
}

export function toolCapabilities(tool) {
  return toolManifest(tool)?.capabilities || [];
}

export function toolHasCapability(tool, capability) {
  return toolCapabilities(tool).includes(capability);
}

export function toolsWithCapability(capability) {
  return TOOLS.filter(tool => toolHasCapability(tool, capability));
}

// path: prefer local file path; id: prefer stable session id (e.g. database agents).
export function toolReferenceKind(tool) {
  return toolManifest(tool)?.reference_kind === "id" ? "id" : "path";
}

export async function loadTools() {
  try {
    const list = await rpc("tools");
    hydrateTools(list);
    try { localStorage.setItem(CACHE_KEY, JSON.stringify(list)); }
    catch { /* 配额不足则放弃缓存 */ }
  } catch { /* 引擎不可用时保持现有清单 */ }
}

// 接续命令由引擎 lifecycle 生成(launch descriptor),前端不再拼装
export const resumeDescriptor = (tool, sessionId, cwd) =>
  rpc("resume", { tool, session_id: sessionId, cwd: cwd || "." });
