import { useDeferredValue, useEffect, useMemo, useState } from "react";
import { renderEvent } from "../../shared/contracts/events.js";
import { Caret } from "../../shared/ui/icons.jsx";

const PAGE_SIZE = 50;

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

function Payload({ label, value }) {
  if (!value) return null;
  return <div style={{ marginTop: 6 }}>
    {label && <div style={{ marginBottom: 3, color: "var(--tx5)", fontSize: 10 }}>{label}</div>}
    <pre className="mono fscroll selectable" style={{ margin: 0, maxHeight: 170, overflow: "auto",
      padding: "7px 9px", borderRadius: 6, background: "var(--fill)", color: "var(--tx2b)",
      fontSize: 10.5, lineHeight: 1.55, whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{value}</pre>
  </div>;
}

function CallSide({ title, snapshot, missing, t }) {
  const parts = snapshot?.parts;
  return <div style={{ minWidth: 0 }}>
    <div style={{ display: "flex", alignItems: "baseline", gap: 7 }}>
      <span style={{ color: "var(--tx4)", fontSize: 10.5, fontWeight: 650 }}>{title}</span>
      {snapshot && <span className="mono" style={{ fontSize: 10.5, color: "var(--tx3)",
        overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{snapshot.label}</span>}
    </div>
    {!snapshot && <div style={{ marginTop: 6, padding: "7px 9px", borderRadius: 6,
      border: "1px dashed var(--line4)", color: "var(--tx5)", fontSize: 11 }}>{missing}</div>}
    {snapshot && (parts
      ? <>
        <Payload label={t("migration:preview.differences.params")} value={parts.input} />
        <Payload label={t("migration:preview.differences.result")} value={parts.output} />
      </>
      : <Payload value={snapshot.detail} />)}
  </div>;
}

function IssueCard({ item, t, onLocate }) {
  const [open, setOpen] = useState(false);
  const fidelity = item.fidelity || (item.kind === "dropped" ? "dropped" : "narrated");
  const color = fidelity === "dropped" ? "var(--err)"
    : fidelity === "transformed" ? "var(--accent)" : "var(--warn)";
  const sourceLabel = item.event ? t("migration:preview.differences.sessionLoss")
    : item.source?.kind === "thinking" ? t("migration:preview.differences.thinking")
      : item.source?.label;
  const targetLabel = !item.target ? null
    : item.target.kind === "tool" ? item.target.label
      : t("migration:preview.differences.narration");
  // 只有调用在目标端仍是一次工具调用时,被忽略的字段才真的"丢了";
  // 退化成叙述或整块丢弃时,参数其实都还在文本里,标成丢失反而误导。
  const lost = item.target?.kind === "tool" ? item.ignored_fields || [] : [];
  const note = lost.length ? t("migration:preview.differences.lostFields", { fields: lost.join(", ") })
    : item.event ? renderEvent(item.event)
      : item.reason_code === "tool_transformed" ? null
        : t(`migration:preview.differences.reasons.${item.reason_code}`,
          { tool: item.source?.label, defaultValue: item.reason_code || "" });
  return <article style={{ border: "1px solid var(--line4)", borderRadius: 9,
    background: "var(--surface)", overflow: "hidden" }}>
    <button type="button" onClick={() => setOpen(value => !value)}
      aria-expanded={open}
      style={{ width: "100%", padding: "9px 12px", display: "flex", alignItems: "center", gap: 9,
        border: "none", background: "transparent", color: "inherit", font: "inherit",
        textAlign: "left", cursor: "pointer" }}>
      <span style={{ width: 4, alignSelf: "stretch", minHeight: 26, borderRadius: 4, background: color }} />
      <div style={{ minWidth: 0, flex: 1 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 7, minWidth: 0 }}>
          <span style={{ fontSize: 11, color, fontWeight: 700, flex: "none" }}>
            {t(`migration:preview.differences.${fidelity}`)}
          </span>
          <span className="mono" style={{ fontSize: 11, color: "var(--tx2)", fontWeight: 650,
            overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{sourceLabel}</span>
          {targetLabel && <>
            <span style={{ color: "var(--tx5)", fontSize: 11, flex: "none" }}>→</span>
            <span className="mono" style={{ fontSize: 11, color: "var(--tx3)",
              overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{targetLabel}</span>
          </>}
          {!item.target && !item.event && <>
            <span style={{ color: "var(--tx5)", fontSize: 11, flex: "none" }}>→</span>
            <span style={{ fontSize: 11, color: "var(--tx5)" }}>
              {t("migration:preview.differences.noTarget")}</span>
          </>}
        </div>
        {note && <div style={{ marginTop: 2, color: "var(--tx3b)", fontSize: 11,
          overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{note}</div>}
      </div>
      <Caret open={open} size={9} />
    </button>
    {open && <div style={{ padding: "10px 12px 12px 25px", borderTop: "1px solid var(--line6)" }}>
      <div style={{ display: "grid", gap: 12,
        gridTemplateColumns: "repeat(auto-fit, minmax(232px, 1fr))" }}>
        <CallSide title={t("migration:preview.differences.original")} snapshot={item.source} t={t}
          missing={t("migration:preview.differences.noTarget")} />
        <CallSide title={t("migration:preview.differences.migrated")} snapshot={item.target} t={t}
          missing={t("migration:preview.differences.noTarget")} />
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 10, marginTop: 10 }}>
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

export default function DifferenceReview({ preview, t, onBack, onLocate }) {
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
      <div style={{ display: "flex", alignItems: "center", flexWrap: "wrap", gap: 7 }}>
        <button type="button" className="fbtn" onClick={onBack} style={{ height: 29, fontSize: 11 }}>
          {t("migration:preview.differences.back")}
        </button>
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
