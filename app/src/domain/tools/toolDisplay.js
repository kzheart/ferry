export const ACCENT = "var(--accent)";
export const BIG = 100 * 1024;

export function fmtSize(n) {
  if (!n) return "—";
  if (n < 1024) return n + " B";
  if (n < 1048576) return (n / 1024).toFixed(n < 10240 ? 1 : 0) + " KB";
  return (n / 1048576).toFixed(1) + " MB";
}

export function resumeCommand(tool, id, dir) {
  if (tool === "claude") return `cd ${dir} && claude --resume ${id}`;
  if (tool === "codex") return `codex resume ${id}`;
  return `cd ${dir} && opencode -s ${id}`;
}
