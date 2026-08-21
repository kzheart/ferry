import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOL_NAME } from "../../shared/contracts/tools.js";
import { Caret, Spinner } from "../../shared/ui/icons.jsx";
import { TOOL_LEVEL, toolLevel } from "./agentChatModel.js";
import EntityCards from "./EntityCards.jsx";

const formatDuration = (startedAt, endedAt) => {
  if (!startedAt || !endedAt) return "";
  const seconds = (new Date(endedAt) - new Date(startedAt)) / 1000;
  return seconds >= 0
    ? `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)}s`
    : "";
};

const prettyJson = text => {
  if (typeof text !== "string") return "";
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
};

const preStyle = {
  margin: 0,
  padding: "8px 12px",
  fontSize: 11,
  lineHeight: 1.55,
  color: "var(--tx3)",
  background: "var(--inset)",
  border: "1px solid var(--line5)",
  borderRadius: 9,
  overflow: "auto",
  maxHeight: 260,
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
};
const sectionLabel = {
  fontSize: 10.5,
  color: "var(--tx5)",
  margin: "0 0 3px 2px",
};

const TRACE_ICON = {
  session_search: <><circle cx="11" cy="11" r="7" /><path d="M21 21l-4.3-4.3" /></>,
  session_read: (
    <><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
      <path d="M14 3v5h5M9 13h6M9 17h4" /></>),
  usage: <path d="M3 12h4l3 8 4-16 3 8h4" />,
  migrate: (
    <><path d="M8 3 4 7l4 4M4 7h16" /><path d="M16 21l4-4-4-4M20 17H4" /></>),
  session_edit: (
    <><path d="M12 20h9" /><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z" /></>),
  bash: (
    <><rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M7 9l3 3-3 3M13 15h4" /></>),
  agent_prompt: (
    <><path d="m12 2 1.8 3.7 4.1.6-3 2.9.7 4.1-3.6-1.9-3.6 1.9.7-4.1-3-2.9 4.1-.6z" />
      <path d="M5 19h14M16 16l3 3-3 3" /></>),
};

function TraceIcon({ name, size = 14 }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none"
      stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      {TRACE_ICON[name] || <circle cx="12" cy="12" r="3" />}
    </svg>
  );
}

const traceIconWrap = {
  position: "absolute",
  left: 0,
  top: 3,
  width: 16,
  height: 16,
  display: "grid",
  placeItems: "center",
  background: "var(--bg)",
  color: "var(--tx4)",
};

const tokenTotal = tokens => Object.values(tokens || {})
  .filter(value => typeof value === "number")
  .reduce((total, value) => total + value, 0);

function toolSummary(item, t) {
  const entities = item.entities || [];
  switch (item.name) {
    case "session_search":
      return entities.length ? t("askferry:trace.hits", { n: entities.length }) : "";
    case "session_read": {
      const session = entities.find(entity => entity.type === "Session");
      if (!session) return "";
      const name = session.title || session.sessionId || session.ref || "";
      return session.turn != null
        ? `${name} · ${t("askferry:entity.turn", { n: session.turn })}`
        : name;
    }
    case "usage": {
      const usage = entities.find(entity => entity.type === "UsageSlice");
      return usage
        ? t("askferry:trace.usage", {
          n: tokenTotal(usage.tokens).toLocaleString(),
          s: usage.sessions || 0,
        })
        : "";
    }
    case "migrate": {
      const migration = entities.find(entity => entity.type === "Migration");
      return migration
        ? `${TOOL_NAME[migration.sourceTool] || migration.sourceTool || "?"} → `
          + `${TOOL_NAME[migration.targetTool] || migration.targetTool || "?"}`
        : "";
    }
    case "bash":
      return item.args?.command || "";
    case "session_edit": {
      const edit = entities.find(entity => entity.type === "Edit");
      return edit
        ? t("askferry:entity.edits", {
          count: edit.changes?.length || 1,
          n: edit.changes?.length || 1,
        })
        : "";
    }
    default:
      return "";
  }
}

/** bash 的结果藏在一层 JSON 里,拆出来分块展示;拆不动就回落原样。 */
function shellResult(text) {
  if (typeof text !== "string") return null;
  try {
    const value = JSON.parse(text);
    const inner = value?.result ?? value;
    return typeof inner?.exit_code !== "undefined"
      || typeof inner?.stdout === "string" ? inner : null;
  } catch {
    return null;
  }
}

function ShellResult({ result, t }) {
  const blocks = [
    ["stdout", result.stdout],
    ["stderr", result.stderr],
  ].filter(([, value]) => typeof value === "string" && value.length > 0);
  const failed = result.exit_code !== 0 && result.exit_code !== null;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 11 }}>
        <span style={{ color: failed ? "var(--err-text)" : "var(--tx4)" }}>
          {t("askferry:tool.exitCode", { n: result.exit_code ?? "—" })}
        </span>
        {result.timed_out && (
          <span style={{ color: "var(--err-text)" }}>{t("askferry:tool.timedOut")}</span>)}
      </div>
      {blocks.length === 0 && (
        <pre className="mono selectable" style={preStyle}>
          {t("askferry:tool.noOutput")}
        </pre>)}
      {blocks.map(([label, value]) => (
        <div key={label}>
          <div style={sectionLabel}>{label}</div>
          <pre className="mono selectable" style={{
            ...preStyle,
            color: label === "stderr" ? "var(--err-text)" : preStyle.color,
          }}>{value}</pre>
        </div>
      ))}
      {result.truncated && (
        <div style={{ fontSize: 10.5, color: "var(--tx5)" }}>
          {t("askferry:tool.truncated")}
        </div>)}
    </div>
  );
}

export const AgentToolRow = memo(function AgentToolRow({ item, onNavigate }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const level = toolLevel(item);
  const running = item.status === "running";
  const error = item.status === "error";
  const merged = item.merged;
  const count = merged ? merged.length : 1;
  const resultText = item.result?.text ? prettyJson(item.result.text) : "";
  const shell = item.name === "bash" ? shellResult(item.result?.text) : null;
  const fallbackVerb = t(`askferry:trace.verb.${item.name}`, { defaultValue: item.name });
  const intent = typeof item.args?.intent === "string" ? item.args.intent : null;
  const verb = item.name === "agent_prompt"
    ? t("settings:roles.tool.agent_prompt.label")
    // 分阶段工具按阶段命名:盘点显示成「清理会话」会让用户以为已经删了
    : intent
      ? t(`askferry:trace.verbIntent.${item.name}.${intent}`,
          { defaultValue: fallbackVerb })
      : fallbackVerb;
  const summary = toolSummary(item, t);
  // 实体卡的自动展示仍按工具类别(预览的编辑卡要直接可见),徽章才按阶段
  const showCards = !error && !merged && item.entities?.length > 0
    && (TOOL_LEVEL[item.name] === "mutate" || open);

  return (
    <div style={{ position: "relative", paddingLeft: 24 }}>
      <span style={traceIconWrap}>
        {running ? <Spinner size={12} /> : <TraceIcon name={item.name} />}
      </span>
      <div onClick={() => setOpen(value => !value)}
        style={{ display: "flex", alignItems: "center", gap: 8, minHeight: 22,
          cursor: "default", fontSize: 12 }}>
        <span style={{ color: "var(--tx2)", fontWeight: 500, flex: "none" }}>{verb}</span>
        {level === "mutate" && (
          <span data-tool-level="mutate" style={{ fontSize: 9.5, fontWeight: 600,
            letterSpacing: ".04em", textTransform: "uppercase", color: "var(--warn-text)",
            background: "var(--warn-soft, rgba(240,180,41,.16))",
            padding: "1px 5px", borderRadius: 4,
            flex: "none" }}>
            {t("settings:roles.mutationBadge")}
          </span>)}
        {count > 1 && (
          <span style={{ fontSize: 10.5, fontWeight: 600, color: "var(--acc)",
            background: "var(--acc-soft2)", padding: "0 5px", borderRadius: 4, flex: "none" }}>
            ×{count}</span>)}
        {summary && (
          <span style={{ color: "var(--tx4)", overflow: "hidden",
            textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 0 }}>{summary}</span>)}
        {error && (
          <span style={{ fontSize: 11, color: "var(--err-text)", flex: "none" }}>
            {t("askferry:tool.failed")}</span>)}
        <span style={{ flex: 1 }} />
        {!running && (
          <span style={{ fontSize: 10.5, color: "var(--tx5)", flex: "none" }}>
            {formatDuration(item.startedAt, item.endedAt)}</span>)}
        <Caret open={open} size={8} />
      </div>

      {open && merged && (
        <div style={{ display: "flex", flexDirection: "column", gap: 3,
          margin: "4px 0 4px" }}>
          {merged.map((sub, index) => (
            <div key={sub.callId || index}
              style={{ display: "flex", gap: 8, fontSize: 11.5, color: "var(--tx4)" }}>
              <span style={{ color: "var(--tx5)", flex: "none" }}>{index + 1}.</span>
              <span style={{ overflow: "hidden", textOverflow: "ellipsis",
                whiteSpace: "nowrap", minWidth: 0 }}>
                {toolSummary(sub, t) || t("askferry:tool.noResult")}</span>
              <span style={{ flex: 1 }} />
              <span style={{ color: "var(--tx5)", flex: "none" }}>
                {formatDuration(sub.startedAt, sub.endedAt)}</span>
            </div>
          ))}
        </div>
      )}

      {open && !merged && (
        <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 5 }}>
          <div>
            <div style={sectionLabel}>{t("askferry:tool.args")}</div>
            <pre className="mono selectable" style={preStyle}>
              {JSON.stringify(item.args ?? {}, null, 2)}
            </pre>
          </div>
          {!running && shell && !error ? (
            <ShellResult result={shell} t={t} />
          ) : !running && (
            <div>
              <div style={sectionLabel}>
                {error ? t("askferry:tool.errorResult") : t("askferry:tool.result")}
                {item.result?.truncated && ` · ${t("askferry:tool.truncated")}`}
              </div>
              <pre className="mono selectable" style={{
                ...preStyle,
                color: error ? "var(--err-text)" : preStyle.color,
              }}>
                {resultText || t("askferry:tool.noResult")}
              </pre>
            </div>
          )}
        </div>
      )}
      {showCards && (
        <div style={{ marginTop: 6 }}>
          <EntityCards entities={item.entities} onNavigate={onNavigate} />
        </div>
      )}
    </div>
  );
});

export function AgentToolTrace({ rows, onNavigate }) {
  return (
    <div style={{ position: "relative", display: "flex", flexDirection: "column", gap: 2 }}>
      <span style={{ position: "absolute", left: 7.5, top: 14, bottom: 14, width: 1.5,
        background: "var(--line3)", borderRadius: 1 }} />
      {rows.map((row, index) => (
        <AgentToolRow key={row.callId || index} item={row} onNavigate={onNavigate} />))}
    </div>
  );
}
