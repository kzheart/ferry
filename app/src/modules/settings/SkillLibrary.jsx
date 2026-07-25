// 技能页左栏上半:已导入技能。只有这里的条目才是 Ferry 的技能,能被设为通用、被角色挂载。
import { useTranslation } from "react-i18next";
import { isGlobal } from "./skillModel.js";

export default function SkillLibrary({ skills, global, selectedId, onSelect }) {
  const { t } = useTranslation();
  if (skills.length === 0) {
    return (
      <div style={{ padding: "10px 10px 14px", fontSize: 11.5, color: "var(--tx5)",
        lineHeight: 1.6 }}>
        {t("settings:skills.emptyLibrary")}
      </div>
    );
  }
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      {skills.map(skill => {
        const on = selectedId === `installed:${skill.id}`;
        return (
          <button key={skill.id} className={on ? undefined : "hov-item"}
            onClick={() => onSelect(`installed:${skill.id}`)}
            style={{ display: "flex", alignItems: "center", gap: 8, border: "none",
              borderRadius: 8, padding: "7px 8px", textAlign: "left", cursor: "default",
              fontFamily: "inherit", background: on ? "var(--seg-on)" : "transparent" }}>
            <span style={{ minWidth: 0, flex: 1 }}>
              <span style={{ display: "flex", alignItems: "center", gap: 5 }}>
                <span style={{ fontSize: 12.5, fontWeight: on ? 650 : 600, minWidth: 0,
                  overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
                  color: skill.broken ? "var(--err-text)" : on ? "var(--tx1)" : "var(--tx2b)" }}>
                  {skill.name}</span>
                {isGlobal(skill.id, global) && (
                  <span style={{ flex: "none", fontSize: 9.5, fontWeight: 650,
                    padding: "1px 5px", borderRadius: 4, background: "var(--acc-soft3)",
                    color: "var(--acc-text)" }}>
                    {t("settings:skills.globalBadge")}</span>)}
              </span>
              <span style={{ display: "block", marginTop: 1, fontSize: 10.5,
                color: "var(--tx5)", overflow: "hidden", textOverflow: "ellipsis",
                whiteSpace: "nowrap" }}>
                {skill.broken ? t("settings:skills.broken") : skill.id}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
