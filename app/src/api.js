// 引擎桥:所有能力经 Tauri command 调 Python 引擎(engine/api.py rpc)
import { invoke } from "@tauri-apps/api/core";

const inTauri = () => !!window.__TAURI_INTERNALS__;

export async function rpc(method, params) {
  const request = JSON.stringify({ method, params: params || {} });
  const raw = inTauri()
    ? await invoke("engine_rpc", { request })
    : await (await fetch("/api/rpc", { method: "POST", body: request })).text();
  const j = JSON.parse(raw);
  if (!j.ok) throw new Error(j.error || "引擎调用失败");
  return j.result;
}

export const openTerminal = (command) =>
  inTauri() ? invoke("open_terminal", { command }) : Promise.resolve();

export const TOOL_NAME = { claude: "Claude Code", codex: "Codex CLI", opencode: "OpenCode" };
export const TOOLS = ["claude", "codex", "opencode"];
export const BIG = 100 * 1024;
// 强调色由设置写入根节点的 CSS 变量(见 settings.js),这里始终引用变量
export const ACCENT = "var(--accent)";

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
  if (d < 30 * 86400e3) return Math.floor(d / 7 / 86400e3) + " 周前";
  const t = new Date(ms);
  return `${t.getMonth() + 1} 月 ${t.getDate()} 日`;
}

// 时间分桶:今天 / 昨天 / 最近 7 天 / 最近 30 天 / 更早
export const BUCKETS = [
  ["today", "今天"], ["yesterday", "昨天"], ["last7", "最近 7 天"],
  ["last30", "最近 30 天"], ["earlier", "更早"],
];

export function bucketOf(ms) {
  if (!ms) return "earlier";
  const now = new Date();
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (ms >= midnight) return "today";
  if (ms >= midnight - 86400e3) return "yesterday";
  if (ms >= midnight - 6 * 86400e3) return "last7";
  if (ms >= midnight - 29 * 86400e3) return "last30";
  return "earlier";
}

export function repoOf(dir) {
  if (!dir) return "";
  const parts = String(dir).split("/").filter(Boolean);
  return parts[parts.length - 1] || dir;
}

export function resumeCommand(tool, id, dir) {
  if (tool === "claude") return `cd ${dir} && claude --resume ${id}`;
  if (tool === "codex") return `codex resume ${id}`;
  return `cd ${dir} && opencode -s ${id}`;
}

export function sessionRef(s) {
  return s.tool === "opencode" ? s.id : (s.path || s.id);
}

// 把 show() 的消息序列折成"轮"(用户消息起新轮)
export function toRounds(messages) {
  const rounds = [];
  let cur = null;
  for (const m of messages || []) {
    const texts = m.blocks.filter(b => b.kind === "text" && b.text.trim());
    if (m.role === "user" && texts.length) {
      cur = { n: rounds.length + 1, user: texts.map(t => t.text).join("\n"),
              uuid: m.uuid, index: m.index, ai: [], tools: [] };
      rounds.push(cur);
      continue;
    }
    if (!cur) {
      cur = { n: 1, user: "", uuid: null, index: m.index, ai: [], tools: [] };
      rounds.push(cur);
    }
    if (m.role === "assistant") texts.forEach(t => cur.ai.push(t.text));
    m.blocks.forEach(b => { if (b.kind === "tool") cur.tools.push(b); });
  }
  return rounds;
}

export function histStatus(h) {
  if (h.rolled_back) return "已回滚";
  if (h.probe && !h.probe.ok) return "失败";
  if (h.dry_run) return "预演";
  if (h.session_id) return "成功";
  return "失败";
}
