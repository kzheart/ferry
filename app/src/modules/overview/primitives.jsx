// 总览页共用的卡片外壳、数字样式与图表配色,供 Overview 与各面板复用。
import { TOOLS } from "../../shared/contracts/tools.js";

export const card = { background: "var(--surface)", border: "1px solid var(--line)",
  borderRadius: 10, boxShadow: "var(--shadow)" };
export const num = { fontVariantNumeric: "tabular-nums" };

export const CHART = ["var(--c1)", "var(--c2)", "var(--c3)", "var(--c4)"];
const TOOL_COLOR = {
  claude: "var(--t-claude)",
  codex: "var(--t-codex)",
  opencode: "var(--t-opencode)",
  pi: "var(--c4)",
  grok: "var(--tx2)",
  cursor: "var(--t-cursor)",
};
export const toolColor = tool => {
  const index = TOOLS.indexOf(tool);
  return TOOL_COLOR[tool] || CHART[(index < 0 ? 0 : index) % CHART.length];
};

export function Card({ title, sub, extra, fill, children }) {
  return (
    <div style={fill ? { ...card, height: "100%", display: "flex", flexDirection: "column" } : card}>
      <div style={{ display: "flex", alignItems: "baseline", gap: 10, padding: "13px 15px 0" }}>
        <h2 style={{ margin: 0, fontSize: 12, fontWeight: 600, letterSpacing: ".02em", color: "var(--tx2)" }}>{title}</h2>
        {sub && <span style={{ fontSize: 11, color: "var(--tx4b)" }}>{sub}</span>}
        {extra && <><div style={{ flex: 1 }} />{extra}</>}
      </div>
      <div style={fill ? { padding: "13px 15px 15px", flex: 1, display: "flex", flexDirection: "column" }
        : { padding: "13px 15px 15px" }}>{children}</div>
    </div>
  );
}

export function Section({ title, note }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 10, margin: "4px 0 -8px" }}>
      <h3 style={{ margin: 0, fontSize: 12, fontWeight: 600, color: "var(--tx2)", letterSpacing: ".01em" }}>{title}</h3>
      <div style={{ flex: 1, height: 1, background: "var(--line)" }} />
      {note && <span style={{ fontSize: 11, color: "var(--tx4b)" }}>{note}</span>}
    </div>
  );
}
