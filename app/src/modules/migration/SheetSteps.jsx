// 面板顶部的步骤条。迁移五步、交接三步,由调用方给 order。
import { ACCENT } from "../../shared/ui/toolDisplay.js";

export default function StepsHeader({ step, order, t }) {
  const labels = {
    target: t("migration:steps.target"),
    impact: t("migration:steps.impact"),
    preview: t("migration:steps.preview"),
    confirm: t("migration:steps.confirm"),
    result: step === "writing" ? t("migration:steps.writing") : t("migration:steps.result"),
  };
  const cur = order.indexOf(step === "writing" ? "result" : step);
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 7, marginLeft: 6 }}>
      {order.map((s, i) => (
        <span key={s} style={{ display: "inline-flex", alignItems: "center", gap: 7 }}>
          <span style={{ fontSize: 11, fontWeight: 600,
            color: i === cur ? ACCENT : i < cur ? "var(--tx3)" : "var(--line-strong)" }}>{labels[s]}</span>
          {i < order.length - 1 && <span style={{ color: "var(--line-strong)", fontSize: 11 }}>›</span>}
        </span>
      ))}
    </div>
  );
}
