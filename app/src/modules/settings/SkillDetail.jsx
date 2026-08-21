// 技能详情:已导入的给「设为通用 / 删除」,候选的只给「导入」。
// 候选详情里不出现「设为通用」——没导入就没有可配置的东西。
import { useTranslation } from "react-i18next";
import Markdown from "../../shared/ui/Markdown.jsx";
import { Toggle } from "./parts.jsx";
import { formatSkillSize } from "./skillModel.js";

const metaStyle = { fontSize: 11.5, color: "var(--tx5)", marginTop: 4 };

function Action({ label, onClick, danger, busy }) {
  return (
    <button className="hov-item" onClick={onClick} disabled={busy}
      style={{ height: 28, padding: "0 12px", borderRadius: 8, flex: "none",
        border: `1px solid ${danger ? "var(--err-line)" : "var(--line4)"}`,
        background: "transparent", fontFamily: "inherit", fontSize: 12, fontWeight: 600,
        color: danger ? "var(--err-text)" : "var(--tx2)", cursor: "default" }}>
      {label}</button>
  );
}

export default function SkillDetail({
  skill, candidate, isGlobalSkill, body, busy,
  onToggleGlobal, onDelete, onImport,
}) {
  const { t } = useTranslation();
  if (!skill && !candidate) {
    return (
      <div style={{ padding: "40px 24px", fontSize: 12, color: "var(--tx5)" }}>
        {t("settings:skills.pickOne")}
      </div>
    );
  }
  const title = skill ? skill.name : candidate.name;
  const description = skill ? skill.description : candidate.description;

  return (
    <div style={{ maxWidth: 640, margin: "0 auto", padding: "18px 0 24px" }}>
      <div style={{ display: "flex", alignItems: "flex-start", gap: 12,
        paddingBottom: 16, borderBottom: "1px solid var(--line4)" }}>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ fontSize: 17, fontWeight: 600, color: "var(--tx1)",
            letterSpacing: "-.01em" }}>{title}</div>
          <div style={metaStyle}>
            {skill
              ? [skill.id, formatSkillSize(skill.bytes),
                t("settings:skills.fileCount", { n: skill.files }),
                skill.originLabel || t("settings:skills.originManual")]
                .filter(Boolean).join(" · ")
              : candidate.path}
          </div>
        </div>
        <div style={{ flex: "none", display: "flex", alignItems: "center", gap: 8 }}>
          {candidate && (
            <Action busy={busy} onClick={onImport}
              label={candidate.installedId
                ? t("settings:skills.reimport")
                : t("settings:skills.import")} />)}
          {skill && (
            <Action busy={busy} danger onClick={onDelete}
              label={t("settings:skills.delete")} />)}
        </div>
      </div>

      {description && (
        <div style={{ fontSize: 12.5, color: "var(--tx3)", lineHeight: 1.65,
          margin: "14px 0 0" }}>{description}</div>)}

      {/* 只有已导入的技能才谈得上「通用」:候选还不在库里,没有可被角色引用的 id */}
      {skill && (
        <div style={{ display: "flex", alignItems: "center", gap: 12, marginTop: 16,
          padding: "12px 14px", border: "1px solid var(--line4)", borderRadius: 11,
          background: "var(--surface)" }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: "var(--tx1)" }}>
              {t("settings:skills.globalTitle")}</div>
            <div style={{ fontSize: 11, color: "var(--tx4)", marginTop: 2 }}>
              {t("settings:skills.globalHint")}</div>
          </div>
          <Toggle on={!!isGlobalSkill} onChange={onToggleGlobal} />
        </div>
      )}

      {skill && (
        <div style={{ marginTop: 18 }}>
          <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx5)",
            letterSpacing: ".05em", marginBottom: 8 }}>SKILL.md</div>
          <div style={{ border: "1px solid var(--line4)", borderRadius: 11,
            background: "var(--surface)", padding: "12px 16px", fontSize: 12.5 }}>
            {body ? <Markdown text={body} />
              : <span style={{ color: "var(--tx5)" }}>{t("settings:skills.loading")}</span>}
          </div>
        </div>
      )}
    </div>
  );
}
