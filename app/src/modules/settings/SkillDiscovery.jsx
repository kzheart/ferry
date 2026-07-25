// 技能页左栏下半:外部目录里扫到的候选。
// 候选不是 Ferry 的技能——运行时永远不会读它们,必须点「导入」复制进库才生效。
// 来源默认折叠:一个共享仓库就可能有几十个候选,全铺开会把上面的「我的技能」挤没。
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { groupBySource } from "./skillModel.js";

const Chevron = ({ open }) => (
  <svg width="9" height="9" viewBox="0 0 12 12" aria-hidden="true"
    style={{ flex: "none", transform: open ? "rotate(90deg)" : "none",
      transition: "transform .12s ease" }}>
    <path d="M4.5 2.5 8.5 6l-4 3.5" fill="none" stroke="currentColor"
      strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

function SourceHeader({ source, count, open, onToggle, onRemoveSource }) {
  const { t } = useTranslation();
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 4,
      padding: "8px 8px 2px" }}>
      <button className="hov-item" onClick={onToggle} disabled={!count}
        aria-expanded={open}
        title={t(open ? "settings:skills.collapseSource" : "settings:skills.expandSource")}
        style={{ display: "flex", alignItems: "center", gap: 5, flex: 1, minWidth: 0,
          border: "none", borderRadius: 6, padding: "2px 4px", margin: "0 -4px",
          background: "transparent", textAlign: "left", cursor: "default",
          fontFamily: "inherit", color: "var(--tx5)" }}>
        <Chevron open={open} />
        <span style={{ fontSize: 10, fontWeight: 700, letterSpacing: ".04em",
          minWidth: 0, overflow: "hidden", textOverflow: "ellipsis",
          whiteSpace: "nowrap" }}>
          {source.builtin ? source.label : source.path}</span>
        {count > 0 && (
          <span style={{ flex: "none", fontSize: 9.5, fontWeight: 650,
            padding: "0 4px", borderRadius: 4, background: "var(--fill3)" }}>
            {count}</span>)}
      </button>
      {!source.available && (
        <span style={{ flex: "none", fontSize: 9.5, color: "var(--tx5)" }}>
          {t("settings:skills.sourceMissing")}</span>)}
      {!source.builtin && (
        <button className="hov" title={t("settings:skills.removeSource")}
          onClick={() => onRemoveSource(source.id)}
          style={{ border: "none", background: "transparent", padding: 0,
            color: "var(--tx5)", cursor: "default", fontSize: 13, lineHeight: 1,
            flex: "none" }}>×</button>)}
    </div>
  );
}

function CandidateRow({ candidate, on, onSelect }) {
  const { t } = useTranslation();
  return (
    <button className={on ? undefined : "hov-item"}
      onClick={() => onSelect(`candidate:${candidate.candidateId}`)}
      style={{ display: "flex", alignItems: "center", gap: 8, border: "none",
        borderRadius: 8, padding: "6px 8px", textAlign: "left", cursor: "default",
        width: "100%", fontFamily: "inherit",
        background: on ? "var(--seg-on)" : "transparent" }}>
      <span style={{ minWidth: 0, flex: 1 }}>
        <span style={{ display: "block", fontSize: 12, fontWeight: on ? 650 : 600,
          color: on ? "var(--tx1)" : "var(--tx3b)", overflow: "hidden",
          textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {candidate.name}</span>
      </span>
      {candidate.installedId && (
        <span style={{ flex: "none", fontSize: 9.5, fontWeight: 650,
          padding: "1px 5px", borderRadius: 4, background: "var(--fill3)",
          color: "var(--tx5)" }}>
          {t("settings:skills.importedBadge")}</span>)}
    </button>
  );
}

export default function SkillDiscovery({
  candidates, sources, selectedId, onSelect, onRemoveSource,
}) {
  const [opened, setOpened] = useState(() => new Set());
  const groups = groupBySource(candidates, sources);
  const toggle = (id) => setOpened(current => {
    const next = new Set(current);
    if (!next.delete(id)) next.add(id);
    return next;
  });
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      {groups.map(({ source, items }) => {
        // 选中的候选所在的组强制展开,否则点完详情后左栏找不到它了
        const holdsSelection = items.some(
          item => selectedId === `candidate:${item.candidateId}`);
        const open = opened.has(source.id) || holdsSelection;
        return (
          <div key={source.id}>
            <SourceHeader source={source} count={items.length} open={open}
              onToggle={() => toggle(source.id)} onRemoveSource={onRemoveSource} />
            {open && items.map(candidate => (
              <CandidateRow key={candidate.candidateId} candidate={candidate}
                on={selectedId === `candidate:${candidate.candidateId}`}
                onSelect={onSelect} />
            ))}
          </div>
        );
      })}
    </div>
  );
}
