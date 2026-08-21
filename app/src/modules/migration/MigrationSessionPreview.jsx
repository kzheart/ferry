import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import Markdown from "../../shared/ui/Markdown.jsx";
import { Caret } from "../../shared/ui/icons.jsx";
import DifferenceReview from "./MigrationDifferences.jsx";

const MAX_TOOL_OUTPUT = 5000;
const MESSAGE_COLLAPSE_LIMIT = 1800;

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
        <span className="mono" style={{ fontSize: 11, fontWeight: 600, flex: 1 }}>{block.name}</span>
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
        color: "var(--accent)", font: "inherit", fontSize: 11, fontWeight: 600, cursor: "pointer" }}>
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
          <span className="mono" style={{ color: "var(--tx3)", fontWeight: 600 }}>{counts.total}</span>
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
