// 角色详情里的技能多选。只列已导入的技能——候选不在库里,给不出可引用的 id。
import { useTranslation } from "react-i18next";
import { Card } from "./parts.jsx";
import { isGlobal, missingSkillIds } from "./skillModel.js";

const rowBase = {
  display: "flex", alignItems: "center", gap: 11, padding: "11px 15px",
  textAlign: "left", width: "100%", border: "none", background: "transparent",
  fontFamily: "inherit", cursor: "default",
};

function Check({ on, locked }) {
  return (
    <span style={{ width: 16, height: 16, flex: "none", borderRadius: 5,
      border: `1.4px solid ${on ? "var(--accent)" : "var(--line3)"}`,
      background: on ? "var(--accent)" : "transparent",
      opacity: locked ? .55 : 1,
      display: "grid", placeItems: "center" }}>
      {on && (
        <svg viewBox="0 0 16 16" style={{ width: 11, height: 11 }} aria-hidden>
          <path d="M3.4 8.4 6.5 11.5 12.6 5" fill="none" stroke="#fff" strokeWidth="2"
            strokeLinecap="round" strokeLinejoin="round" />
        </svg>)}
    </span>
  );
}

export default function RoleSkillPicker({ skills, global, value, onChange }) {
  const { t } = useTranslation();
  const selected = value || [];
  const missing = missingSkillIds(selected, skills);

  if (skills.length === 0 && missing.length === 0) {
    return (
      <Card>
        <div style={{ padding: "16px 15px", fontSize: 11.5, color: "var(--tx5)" }}>
          {t("settings:skills.roleEmpty")}
        </div>
      </Card>
    );
  }

  const toggle = id => onChange(
    selected.includes(id) ? selected.filter(item => item !== id) : [...selected, id]);

  return (
    <Card>
      {skills.map((skill, index) => {
        // 全局技能对所有角色生效,这里锁死勾选态——两处都能改会让语义打架
        const locked = isGlobal(skill.id, global);
        const on = locked || selected.includes(skill.id);
        return (
          <button key={skill.id} type="button" className={locked ? undefined : "hov-item"}
            disabled={locked || skill.broken}
            onClick={() => toggle(skill.id)}
            style={{ ...rowBase,
              borderTop: index === 0 ? "none" : "1px solid var(--line6)",
              opacity: skill.broken ? .5 : 1 }}>
            <Check on={on && !skill.broken} locked={locked} />
            <span style={{ minWidth: 0, flex: 1 }}>
              <span style={{ display: "block", fontSize: 13, fontWeight: 600,
                color: "var(--tx1)" }}>{skill.name}</span>
              {skill.description && (
                <span style={{ display: "block", fontSize: 11, color: "var(--tx4)",
                  marginTop: 2, overflow: "hidden", textOverflow: "ellipsis",
                  whiteSpace: "nowrap" }}>{skill.description}</span>)}
            </span>
            {locked && (
              <span style={{ flex: "none", fontSize: 9.5, fontWeight: 600, padding: "1px 6px",
                borderRadius: 4, background: "var(--acc-soft3)", color: "var(--acc-text)" }}>
                {t("settings:skills.globalBadge")}</span>)}
          </button>
        );
      })}
      {missing.map(id => (
        <button key={id} type="button" className="hov-item" onClick={() => toggle(id)}
          style={{ ...rowBase, borderTop: "1px solid var(--line6)" }}>
          <Check on />
          <span style={{ minWidth: 0, flex: 1, fontSize: 13, fontWeight: 600,
            color: "var(--tx4)" }}>{id}</span>
          <span style={{ flex: "none", fontSize: 9.5, fontWeight: 600, padding: "1px 6px",
            borderRadius: 4, background: "var(--err-bg)", color: "var(--err-text)" }}>
            {t("settings:skills.roleMissing")}</span>
        </button>
      ))}
    </Card>
  );
}
