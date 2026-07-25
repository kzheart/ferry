// 技能页左栏下半:外部目录里扫到的候选。
// 候选不是 Ferry 的技能——运行时永远不会读它们,必须点「导入」复制进库才生效。
import { useTranslation } from "react-i18next";
import { groupBySource } from "./skillModel.js";

export default function SkillDiscovery({
  candidates, sources, selectedId, onSelect, onRemoveSource,
}) {
  const { t } = useTranslation();
  const groups = groupBySource(candidates, sources);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      {groups.map(({ source, items }) => (
        <div key={source.id}>
          <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "8px 8px 4px" }}>
            <span style={{ fontSize: 10, fontWeight: 700, letterSpacing: ".04em",
              color: "var(--tx5)", minWidth: 0, overflow: "hidden",
              textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {source.builtin ? source.label : source.path}</span>
            {!source.available && (
              <span style={{ flex: "none", fontSize: 9.5, color: "var(--tx5)" }}>
                {t("settings:skills.sourceMissing")}</span>)}
            <span style={{ flex: 1 }} />
            {!source.builtin && (
              <button className="hov" title={t("settings:skills.removeSource")}
                onClick={() => onRemoveSource(source.id)}
                style={{ border: "none", background: "transparent", padding: 0,
                  color: "var(--tx5)", cursor: "default", fontSize: 13, lineHeight: 1,
                  flex: "none" }}>×</button>)}
          </div>
          {items.map(candidate => {
            const on = selectedId === `candidate:${candidate.candidateId}`;
            return (
              <button key={candidate.candidateId} className={on ? undefined : "hov-item"}
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
          })}
        </div>
      ))}
    </div>
  );
}
