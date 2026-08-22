// 「目标」步:选迁移目标工具。
//
// 从 MigrateSheet 拆出来只为让主文件保持可读;列表本身没有分支——未装的目标也列
// 出来并置灰,让「为什么它不能选」有答案。
import { useTranslation } from "react-i18next";
import { TOOL_NAME } from "../../shared/contracts/tools.js";
import { ACCENT } from "../../shared/ui/toolDisplay.js";
import { ToolIcon } from "../../shared/ui/icons.jsx";

function Radio({ on }) {
  return (
    <span style={{ width: 18, height: 18, borderRadius: "50%", flex: "none",
      border: `2px solid ${on ? ACCENT : "var(--line-strong)"}`, display: "inline-flex",
      alignItems: "center", justifyContent: "center" }}>
      <span style={{ width: 9, height: 9, borderRadius: "50%",
        background: on ? ACCENT : "transparent" }} />
    </span>
  );
}

export default function TransferTargetStep({
  meta,
  scopeLabel,
  targets,
  target,
  onTarget,
  env,
}) {
  const { t } = useTranslation();
  const installed = tool => env?.[tool]?.installed;

  return (
    <>
      <div style={{ fontSize: 13, color: "var(--tx3b)", marginBottom: 6 }}>
        {t("migration:target.sourceSession")}{" "}
        <b style={{ color: "var(--tx2)" }}>{meta.title || meta.id}</b> · {scopeLabel}</div>
      <div style={{ fontSize: 12, color: "var(--tx4)", marginBottom: 14 }}>
        {t("migration:target.chooseHint")}</div>
      {!targets.length ? (
        <div style={{ fontSize: 12, color: "var(--tx3b)", lineHeight: 1.55,
          border: "1px solid var(--line3)", borderRadius: 10, padding: "14px 16px" }}>
          {t("migration:target.noTargets")}
        </div>
      ) : targets.map(tool => {
        const on = target === tool;
        const usable = installed(tool);
        return (
          <div key={tool} onClick={() => { if (usable) onTarget(tool); }}
            style={{ display: "flex", alignItems: "center", gap: 12, padding: "13px 14px",
              border: `1.5px solid ${on ? ACCENT : "var(--line3)"}`,
              background: on ? "var(--acc-soft4)" : "var(--surface)",
              borderRadius: 10, marginBottom: 9, cursor: usable ? "pointer" : "not-allowed",
              opacity: usable ? 1 : 0.55 }}>
            <ToolIcon tool={tool} size={32} />
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: "var(--tx2)" }}>
                {TOOL_NAME[tool]}
              </div>
              <div style={{ fontSize: 11, color: "var(--tx4)" }}>
                {usable ? t("migration:target.installedMeta", { tool })
                  : t("migration:target.notInstalled")}
              </div>
            </div>
            <Radio on={on} />
          </div>
        );
      })}
    </>
  );
}
