import { useDeferredValue, useEffect, useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { renderEvent } from "../../shared/contracts/events.js";
import Markdown from "../../components/ui/Markdown.jsx";
import { Caret } from "../../components/ui/icons.jsx";

const MAX_TOOL_OUTPUT = 5000;
const MESSAGE_COLLAPSE_LIMIT = 1800;
const PAGE_SIZE = 50;

const clipped = (value, limit) => {
  const text = String(value || "");
  return text.length > limit ? `${text.slice(0, limit)}\n…` : text;
};

function PreviewToolCard({ block }) {
  const [open, setOpen] = useState(false);
  const input = typeof block.input === "string" ? block.input : JSON.stringify(block.input || {}, null, 2);
  return (
    <div style={{ margin: "8px 0", border: "1px solid var(--line3)", borderRadius: 8,
      overflow: "hidden", background: "var(--fill)" }}>
      <button type="button" onClick={() => setOpen(value => !value)}
        aria-expanded={open}
        style={{ width: "100%", minHeight: 34, padding: "6px 10px", display: "flex", alignItems: "center", gap: 8,
          border: "none", background: "transparent", color: "var(--tx2)", font: "inherit", textAlign: "left", cursor: "pointer" }}>
        <Caret open={open} size={9} />
        <span className="mono" style={{ fontSize: 11, fontWeight: 650, flex: 1 }}>{block.name}</span>
      </button>
      {open && <div style={{ padding: "9px 11px", borderTop: "1px solid var(--line5)", background: "var(--surface)" }}>
        <pre className="mono fscroll selectable" style={{ margin: 0, maxHeight: 105, overflow: "auto",
          color: "var(--tx2b)", fontSize: 10.5, lineHeight: 1.55, whiteSpace: "pre-wrap" }}>{clipped(input, MAX_TOOL_OUTPUT)}</pre>
        {block.output && <pre className="mono fscroll selectable" style={{ margin: "9px 0 0", maxHeight: 132, overflow: "auto",
          color: "var(--tx2b)", fontSize: 10.5, lineHeight: 1.55, whiteSpace: "pre-wrap",
          borderTop: "1px solid var(--line5)", paddingTop: 9 }}>{clipped(block.output, MAX_TOOL_OUTPUT)}</pre>}
      </div>}
    </div>
  );
}

function PreviewTextBlock({ block, t, user }) {
  const text = String(block.text || "");
  const shouldCollapse = text.length > MESSAGE_COLLAPSE_LIMIT;
  const [expanded, setExpanded] = useState(false);
  const visibleText = shouldCollapse && !expanded ? `${text.slice(0, MESSAGE_COLLAPSE_LIMIT)}\n…` : text;
  return <div>
    {user ? <div style={{ whiteSpace: "pre-wrap", lineHeight: 1.65, fontSize: 13 }}>{visibleText}</div>
      : <div className="fdel-text"><Markdown text={visibleText} /></div>}
    {shouldCollapse && <button type="button" onClick={() => setExpanded(value => !value)} className="hov-ghost"
      style={{ marginTop: 7, padding: "3px 7px", border: "none", borderRadius: 5, background: "transparent",
        color: "var(--accent)", font: "inherit", fontSize: 11, fontWeight: 650, cursor: "pointer" }}>
      {expanded ? t("migration:preview.targetSession.collapseMessage") : t("migration:preview.targetSession.expandMessage")}
    </button>}
  </div>;
}

function PreviewBlock({ block, t, user = false }) {
  if (block.kind === "tool") return <PreviewToolCard block={block} />;
  return <PreviewTextBlock block={block} t={t} user={user} />;
}

function toRounds(messages) {
  const rounds = [];
  for (const message of messages || []) {
    const n = message.round_index || rounds.length + 1;
    let current = rounds[rounds.length - 1];
    if (!current || current.n !== n) {
      current = { n, user: [], reply: [] };
      rounds.push(current);
    }
    if (message.role === "user") current.user.push(...(message.blocks || []));
    else current.reply.push(...(message.blocks || []));
  }
  return rounds;
}

function PreviewRound({ round, t, anchorId, requestedRoundId, highlighted }) {
  const [stepsOpen, setStepsOpen] = useState(false);
  let finalIndex = -1;
  round.reply.forEach((block, index) => { if (block.kind === "text") finalIndex = index; });
  const steps = round.reply.filter((_, index) => index !== finalIndex);
  const final = finalIndex >= 0 ? round.reply[finalIndex] : null;
  useEffect(() => {
    if (requestedRoundId === anchorId) setStepsOpen(true);
  }, [requestedRoundId, anchorId]);
  return (
    <div id={anchorId} style={{ margin: "10px 0 24px", scrollMarginTop: 14,
      borderRadius: 10, transition: "background 180ms ease",
      background: highlighted ? "var(--acc-soft4)" : "transparent" }}>
      {round.user.length > 0 && <div style={{ display: "flex", justifyContent: "flex-end" }}>
        <div style={{ width: "fit-content", maxWidth: "82%", background: "var(--fill4)", color: "var(--tx1b)",
          padding: "9px 14px", borderRadius: 16, border: "1px solid var(--line4)", overflowWrap: "break-word" }}>
          {round.user.map((block, index) => <div key={block.key || index} style={{ marginTop: index ? 10 : 0 }}>
            <PreviewBlock block={block} t={t} user />
          </div>)}
        </div>
      </div>}
      {steps.length > 0 && <div style={{ margin: "8px 0" }}>
        <button type="button" onClick={() => setStepsOpen(value => !value)} className="hov-ghost"
          aria-expanded={stepsOpen}
          style={{ display: "inline-flex", alignItems: "center", gap: 6, padding: "3px 8px 3px 4px",
            border: "none", borderRadius: 6, background: "transparent", color: "var(--tx4)",
            font: "inherit", fontSize: 12, cursor: "pointer" }}>
          <Caret open={stepsOpen} size={9} />
          <span>{t("migration:preview.targetSession.stepCount", { n: steps.length })}</span>
        </button>
        {stepsOpen && <div style={{ marginLeft: 18, marginTop: 2, borderLeft: "2px solid var(--line5)", paddingLeft: 13 }}>
          {steps.map((block, index) => <div key={block.key || index} style={{ margin: "7px 0" }}>
            <PreviewBlock block={block} t={t} />
          </div>)}
        </div>}
      </div>}
      {final && <PreviewBlock block={final} t={t} />}
    </div>
  );
}

function PreviewThread({ node, t, prefix, nested = false, requestedRoundId, highlighted }) {
  if (!node) return null;
  const rounds = toRounds(node.messages);
  return (
    <section style={nested ? { marginTop: 22, marginLeft: 18, paddingLeft: 15, borderLeft: "2px solid var(--line5)" } : undefined}>
      {rounds.map(round => {
        const anchorId = `${prefix}-${node.key}/r:${round.n}`;
        return <PreviewRound key={round.n} round={round} t={t} anchorId={anchorId}
          requestedRoundId={requestedRoundId} highlighted={highlighted === anchorId} />;
      })}
      {(node.children || []).map((child, index) => <PreviewThread key={child.key || index} node={child} t={t}
        prefix={prefix} nested requestedRoundId={requestedRoundId} highlighted={highlighted} />)}
    </section>
  );
}

const issueText = item => [
  item.node_title, item.node_path, item.role, item.fidelity, item.reason_code,
  item.source?.label, item.source?.summary,
  item.target?.label, item.target?.summary,
].filter(Boolean).join(" ").toLowerCase();

function FilterChip({ active, label, count, color, onClick, disabled = false }) {
  return <button type="button" onClick={onClick}
    aria-pressed={active}
    disabled={disabled}
    style={{ height: 29, padding: "0 10px", display: "inline-flex", alignItems: "center", gap: 6,
      border: `1px solid ${active ? "var(--line-strong)" : "var(--line4)"}`,
      borderRadius: 7, background: active ? "var(--fill4)" : "transparent",
      color: active ? "var(--tx2)" : "var(--tx4)", font: "inherit", fontSize: 11.5,
      fontWeight: active ? 650 : 500, cursor: disabled ? "default" : "pointer",
      opacity: disabled ? 0.8 : 1 }}>
    {color && <span style={{ width: 6, height: 6, borderRadius: "50%", background: color }} />}
    <span>{label}</span>
    <span className="mono" style={{ color: "var(--tx3)" }}>{count}</span>
  </button>;
}

function IssueCard({ item, t, onLocate }) {
  const [open, setOpen] = useState(false);
  const fidelity = item.fidelity || (item.kind === "dropped" ? "dropped" : "narrated");
  const color = fidelity === "dropped" ? "var(--err)"
    : fidelity === "transformed" ? "var(--accent)" : "var(--warn)";
  const sourceLabel = item.event ? t("migration:preview.differences.sessionLoss")
    : item.source?.kind === "thinking" ? t("migration:preview.differences.thinking")
      : item.source?.label;
  const reason = item.event ? renderEvent(item.event)
    : t(`migration:preview.differences.reasons.${item.reason_code}`,
      { tool: item.source?.label, defaultValue: item.reason_code });
  return <article style={{ border: "1px solid var(--line4)", borderRadius: 9,
    background: "var(--surface)", overflow: "hidden" }}>
    <button type="button" onClick={() => setOpen(value => !value)}
      aria-expanded={open}
      style={{ width: "100%", padding: "10px 12px", display: "flex", alignItems: "center", gap: 9,
        border: "none", background: "transparent", color: "inherit", font: "inherit",
        textAlign: "left", cursor: "pointer" }}>
      <span style={{ width: 4, alignSelf: "stretch", minHeight: 30, borderRadius: 4, background: color }} />
      <div style={{ minWidth: 0, flex: 1 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ fontSize: 11, color, fontWeight: 700 }}>
            {t(`migration:preview.differences.${fidelity}`)}
          </span>
          <span className="mono" style={{ fontSize: 11, color: "var(--tx2)", fontWeight: 650,
            overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {sourceLabel}
          </span>
        </div>
        <div style={{ marginTop: 3, color: "var(--tx3b)", fontSize: 11.5,
          overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{reason}</div>
      </div>
      <Caret open={open} size={9} />
    </button>
    {open && <div style={{ padding: "0 12px 12px 25px", borderTop: "1px solid var(--line6)" }}>
      <DetailBlock title={t("migration:preview.differences.original")} value={item.source?.detail} />
      {item.target && <DetailBlock title={t("migration:preview.differences.migrated")} value={item.target.detail} />}
      <div style={{ display: "flex", alignItems: "center", marginTop: 10 }}>
        <span style={{ color: "var(--tx5)", fontSize: 10.5 }}>
          {item.source?.truncated && t("migration:preview.differences.truncated", { n: item.source.char_count })}
        </span>
        <div style={{ flex: 1 }} />
        {item.anchor_id && <button type="button" className="fbtn" onClick={() => onLocate(item.anchor_id)}
          style={{ height: 28, fontSize: 11 }}>{t("migration:preview.differences.viewTurn")}</button>}
        {!item.anchor_id && <span style={{ color: "var(--tx5)", fontSize: 10.5 }}>
          {t("migration:preview.differences.noLocation")}</span>}
      </div>
    </div>}
  </article>;
}

function DetailBlock({ title, value }) {
  if (!value) return null;
  return <div style={{ marginTop: 10 }}>
    <div style={{ marginBottom: 5, color: "var(--tx4)", fontSize: 10.5, fontWeight: 650 }}>{title}</div>
    <pre className="mono fscroll selectable" style={{ margin: 0, maxHeight: 180, overflow: "auto",
      padding: "8px 10px", borderRadius: 7, background: "var(--fill)", color: "var(--tx2b)",
      fontSize: 10.5, lineHeight: 1.55, whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{value}</pre>
  </div>;
}

function groupIssues(items, t) {
  const groups = [];
  for (const item of items) {
    const key = `${item.node_key}:${item.round_index ?? "node"}`;
    let group = groups[groups.length - 1];
    if (!group || group.key !== key) {
      const session = item.node_title || (item.node_path === "0"
        ? t("migration:preview.targetSession.root")
        : t("migration:preview.targetSession.childPath", {
          path: item.node_path.split(".").slice(1).map(value => Number(value) + 1).join("."),
        }));
      group = { key, title: item.round_index
        ? t("migration:preview.differences.groupTitle", { session, n: item.round_index })
        : t("migration:preview.differences.sessionGroupTitle", { session }), items: [] };
      groups.push(group);
    }
    group.items.push(item);
  }
  return groups;
}

function DifferenceReview({ preview, t, onBack, onLocate }) {
  const items = preview.differences?.items || [];
  const counts = preview.differences?.counts || {};
  const [filter, setFilter] = useState("all");
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(PAGE_SIZE);
  const deferredQuery = useDeferredValue(query);
  const normalized = deferredQuery.trim().toLowerCase();
  const filtered = useMemo(() => items.filter(item =>
    (filter === "all" || item.fidelity === filter) &&
    (!normalized || issueText(item).includes(normalized))), [items, filter, normalized]);
  useEffect(() => setLimit(PAGE_SIZE), [filter, normalized]);
  const visible = filtered.slice(0, limit);
  const groups = groupIssues(visible, t);
  return <div style={{ height: "min(560px, calc(100vh - 278px))", minHeight: 310,
    display: "flex", flexDirection: "column" }}>
    <div style={{ flex: "none", padding: "1px 7px 11px", borderBottom: "1px solid var(--line5)" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 9 }}>
        <button type="button" className="fbtn" onClick={onBack} style={{ height: 29, fontSize: 11 }}>
          {t("migration:preview.differences.back")}
        </button>
        <span style={{ color: "var(--tx2)", fontSize: 12.5, fontWeight: 700 }}>
          {t("migration:preview.differences.title")}
        </span>
        <span style={{ color: "var(--tx4)", fontSize: 11 }}>
          {t("migration:preview.differences.summary", {
            exact: counts.exact || 0,
            transformed: counts.transformed || 0,
            lossy: counts.lossy || 0,
            narrated: counts.narrated || 0,
            dropped: counts.dropped || 0,
          })}
        </span>
      </div>
      <div style={{ display: "flex", alignItems: "center", flexWrap: "wrap", gap: 7, marginTop: 10 }}>
        <FilterChip active={filter === "all"} label={t("migration:preview.differences.all")}
          count={counts.total || 0} onClick={() => setFilter("all")} />
        <FilterChip active={false} label={t("migration:preview.differences.exact")}
          count={counts.exact || 0} color="var(--ok)" disabled />
        <FilterChip active={filter === "transformed"} label={t("migration:preview.differences.transformed")}
          count={counts.transformed || 0} color="var(--accent)" onClick={() => setFilter("transformed")} />
        <FilterChip active={filter === "lossy"} label={t("migration:preview.differences.lossy")}
          count={counts.lossy || 0} color="var(--warn)" onClick={() => setFilter("lossy")} />
        <FilterChip active={filter === "narrated"} label={t("migration:preview.differences.narrated")}
          count={counts.narrated || 0} color="var(--warn)" onClick={() => setFilter("narrated")} />
        <FilterChip active={filter === "dropped"} label={t("migration:preview.differences.dropped")}
          count={counts.dropped || 0} color="var(--err)" onClick={() => setFilter("dropped")} />
        <div style={{ flex: 1 }} />
        {items.length > 20 && <input value={query} onChange={event => setQuery(event.target.value)}
          aria-label={t("migration:preview.differences.search")}
          placeholder={t("migration:preview.differences.search")}
          style={{ width: 176, height: 29, padding: "0 9px", border: "1px solid var(--line4)",
            borderRadius: 7, background: "var(--surface)", color: "var(--tx2)", fontSize: 11 }} />}
      </div>
    </div>
    <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "12px 7px 18px" }}>
      {groups.length ? groups.map(group => <section key={group.key} style={{ marginBottom: 17 }}>
        <div style={{ margin: "0 3px 7px", color: "var(--tx4)", fontSize: 10.5, fontWeight: 650 }}>
          {group.title}
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 7 }}>
          {group.items.map(item => <IssueCard key={item.id} item={item} t={t} onLocate={onLocate} />)}
        </div>
      </section>) : <div style={{ padding: "48px 12px", color: "var(--tx4)", fontSize: 12, textAlign: "center" }}>
        {t("migration:preview.differences.empty")}
      </div>}
      {filtered.length > limit && <div style={{ display: "flex", justifyContent: "center", paddingTop: 3 }}>
        <button type="button" className="fbtn" onClick={() => setLimit(value => value + PAGE_SIZE)}
          style={{ height: 30, fontSize: 11 }}>
          {t("migration:preview.differences.loadMore", { n: filtered.length - limit })}
        </button>
      </div>}
    </div>
  </div>;
}

export default function MigrationSessionPreview({ preview }) {
  const { t } = useTranslation();
  const prefix = useId().replaceAll(":", "");
  const [mode, setMode] = useState("messages");
  const [requestedRoundId, setRequestedRoundId] = useState(null);
  const [highlighted, setHighlighted] = useState(null);
  const counts = preview?.differences?.counts || {};
  useEffect(() => {
    if (mode !== "messages" || !requestedRoundId) return undefined;
    const scrollTimer = setTimeout(() => {
      document.getElementById(requestedRoundId)?.scrollIntoView({ behavior: "smooth", block: "center" });
      setHighlighted(requestedRoundId);
    }, 50);
    const highlightTimer = setTimeout(() => setHighlighted(null), 1500);
    return () => {
      clearTimeout(scrollTimer);
      clearTimeout(highlightTimer);
    };
  }, [mode, requestedRoundId]);
  if (!preview?.root) {
    return <div style={{ padding: "28px 12px", color: "var(--tx4)", fontSize: 12, textAlign: "center" }}>
      {t("migration:preview.targetSession.empty")}
    </div>;
  }
  const locate = anchor => {
    setRequestedRoundId(`${prefix}-${anchor}`);
    setMode("messages");
  };
  if (mode === "differences") {
    return <DifferenceReview preview={preview} t={t} onBack={() => setMode("messages")} onLocate={locate} />;
  }
  return (
    <div style={{ height: "min(560px, calc(100vh - 278px))", minHeight: 310,
      display: "flex", flexDirection: "column" }}>
      <div style={{ flex: "none", minHeight: 34, padding: "0 7px 8px",
        display: "flex", alignItems: "center", justifyContent: "flex-end",
        borderBottom: "1px solid var(--line6)" }}>
        {counts.total > 0 ? <button type="button" className="fbtn" onClick={() => setMode("differences")}
          style={{ height: 29, display: "inline-flex", alignItems: "center", gap: 7, fontSize: 11 }}>
          <span style={{ width: 6, height: 6, borderRadius: "50%", background: counts.dropped ? "var(--err)" : "var(--warn)" }} />
          {t("migration:preview.differences.open")}
          <span className="mono" style={{ color: "var(--tx3)", fontWeight: 700 }}>{counts.total}</span>
        </button> : <span style={{ color: "var(--tx5)", fontSize: 11 }}>
          {t("migration:preview.differences.none")}
        </span>}
      </div>
      <div className="fscroll" style={{ flex: 1, minWidth: 0, overflowY: "auto", padding: "4px 7px 18px" }}>
        <PreviewThread node={preview.root} t={t} prefix={prefix}
          requestedRoundId={requestedRoundId} highlighted={highlighted} />
      </div>
    </div>
  );
}
