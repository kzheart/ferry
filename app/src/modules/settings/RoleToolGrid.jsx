// 角色能力区:每个工具一张卡,开关样式一律相同。
// 写操作不在这里另作标记——是否需要审批由运行时决定,卡片只表达"这个角色有没有这项能力"。
import { useTranslation } from "react-i18next";
import { TOOLS } from "./roleForm.js";
import { TOOL_GLYPH, glyph } from "./roleGlyphs.jsx";

export default function RoleToolGrid({ tools, onChange }) {
  const { t } = useTranslation();
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 9 }}>
      {TOOLS.map(name => {
        const on = tools.includes(name);
        return (
          <button key={name} type="button"
            onClick={() => onChange(on ? tools.filter(item => item !== name)
              : [...tools, name])}
            style={{ display: "flex", gap: 10, alignItems: "flex-start", padding: "11px 12px",
              borderRadius: 11, textAlign: "left", fontFamily: "inherit", cursor: "default",
              border: `1px solid ${on ? "var(--acc-line)" : "var(--line4)"}`,
              background: on ? "var(--acc-soft5)" : "var(--surface)" }}>
            <span style={{ width: 28, height: 28, borderRadius: 8, flex: "none",
              display: "inline-flex", alignItems: "center", justifyContent: "center",
              background: on ? "var(--surface)" : "var(--fill4)",
              color: on ? "var(--tx2)" : "var(--tx3b)" }}>
              {glyph(TOOL_GLYPH[name], 15)}
            </span>
            <span style={{ flex: 1, minWidth: 0 }}>
              <span style={{ display: "block", fontSize: 12.5, fontWeight: 650,
                color: "var(--tx1)" }}>
                {t(`settings:roles.tool.${name}.label`)}</span>
              <span style={{ display: "block", marginTop: 3, fontSize: 10.5,
                lineHeight: 1.5, color: "var(--tx5)" }}>
                {t(`settings:roles.tool.${name}.desc`)}</span>
            </span>
            <span style={{ width: 17, height: 17, borderRadius: 5, flex: "none",
              marginTop: 1, display: "inline-flex", alignItems: "center",
              justifyContent: "center",
              border: `1.5px solid ${on ? "var(--accent)" : "var(--tx5)"}`,
              background: on ? "var(--accent)" : "transparent" }}>
              {on && (
                <svg viewBox="0 0 10 10" aria-hidden style={{ width: 10, height: 10 }}>
                  <path d="M1.6 5.2l2.2 2.2 4.6-4.8" fill="none"
                    stroke="var(--accent-fg)" strokeWidth="1.9"
                    strokeLinecap="round" strokeLinejoin="round" />
                </svg>)}
            </span>
          </button>
        );
      })}
    </div>
  );
}
