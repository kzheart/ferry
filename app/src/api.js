// 引擎桥:所有能力经 Tauri command 调 Python 引擎(engine/api.py rpc)
import { invoke } from "@tauri-apps/api/core";

export async function rpc(method, params) {
  const raw = await invoke("engine_rpc", {
    request: JSON.stringify({ method, params: params || {} }),
  });
  const j = JSON.parse(raw);
  if (!j.ok) throw new Error(j.error || "引擎调用失败");
  return j.result;
}

export const openTerminal = (command) => invoke("open_terminal", { command });

export const TOOL_NAME = { claude: "Claude Code", codex: "Codex CLI", opencode: "OpenCode" };
export const TOOL_SHORT = { claude: "CC", codex: "CX", opencode: "OC" };
export const TOOLS = ["claude", "codex", "opencode"];
export const BIG = 100 * 1024;

export function fmtSize(n) {
  if (!n) return "—";
  if (n < 1024) return n + " B";
  if (n < 1048576) return (n / 1024).toFixed(n < 10240 ? 1 : 0) + " KB";
  return (n / 1048576).toFixed(1) + " MB";
}

export function fmtTime(ms) {
  if (!ms) return "—";
  const d = Date.now() - ms;
  if (d < 60e3) return "刚刚";
  if (d < 3600e3) return Math.floor(d / 60e3) + " 分钟前";
  if (d < 86400e3) return Math.floor(d / 3600e3) + " 小时前";
  if (d < 172800e3) {
    const t = new Date(ms);
    return `昨天 ${String(t.getHours()).padStart(2, "0")}:${String(t.getMinutes()).padStart(2, "0")}`;
  }
  if (d < 7 * 86400e3) return Math.floor(d / 86400e3) + " 天前";
  const t = new Date(ms);
  return `${t.getMonth() + 1} 月 ${t.getDate()} 日`;
}

export function resumeCommand(tool, id, dir) {
  if (tool === "claude") return `cd ${dir} && claude --resume ${id}`;
  if (tool === "codex") return `codex resume ${id}`;
  return `cd ${dir} && opencode -s ${id}`;
}
