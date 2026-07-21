// 首次启动:检测到的工具 + 开始扫描
import { useTranslation } from "react-i18next";
import { TOOL_NAME, TOOLS } from "../../api/contract/tools.js";
import { ToolIcon } from "../../components/ui/icons.jsx";
import appIcon from "../../assets/app-icon.png";

export default function FirstRun({ env, scan, onStart }) {
  const { t } = useTranslation();
  return (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
      minHeight: 0, background: "var(--inset)" }}>
      <div style={{ width: 460, background: "var(--surface)", border: "1px solid var(--line3)", borderRadius: 14,
        boxShadow: "var(--shadow-sheet)", padding: "30px 30px 26px",
         }}>
        <img className="noinvert" src={appIcon} alt="Ferry" width={44} height={44} style={{ display: "block" }} />
        <div style={{ fontSize: 20, fontWeight: 650, marginTop: 16, letterSpacing: "-.01em" }}>
          {t("onboarding:welcome.title")}</div>
        <div style={{ fontSize: 13, color: "var(--tx3b)", marginTop: 6, lineHeight: 1.55 }}>
          {t("onboarding:welcome.desc")}</div>
        <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx5)", letterSpacing: ".04em",
          margin: "20px 0 8px" }}>{t("onboarding:welcome.detectedTools")}</div>
        {TOOLS.map(t2 => {
          const info = env?.[t2] || {};
          const tool = scan?.tools?.[t2];
          const installed = info.installed;
          const detect = tool?.ok
            ? t("onboarding:welcome.detectWithSessions", { path: tool.path, count: tool.count })
            : installed ? t("onboarding:welcome.detectInstalled", { version: info.version || "?" })
              : t("onboarding:welcome.detectNotFound");
          return (
            <div key={t2} style={{ display: "flex", alignItems: "center", gap: 11, padding: "10px 12px",
              border: "1px solid var(--line5)", borderRadius: 8, marginBottom: 8 }}>
              <ToolIcon tool={t2} size={26} dot={installed ? "var(--ok)" : "var(--line-strong)"} />
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, color: "var(--tx2)", fontWeight: 600 }}>{TOOL_NAME[t2]}</div>
                <div style={{ fontSize: 11, color: "var(--tx4)" }}>{detect}</div>
              </div>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 11,
                color: installed ? "var(--ok-deep)" : "var(--tx5)", fontWeight: 600 }}>
                <span style={{ width: 7, height: 7, borderRadius: "50%",
                  background: installed ? "var(--ok)" : "var(--line-strong)" }} />
                {installed ? t("onboarding:welcome.badgeInstalled") : t("onboarding:welcome.badgeNotInstalled")}
              </span>
            </div>
          );
        })}
        <button className="fbtn-primary" onClick={onStart}
          style={{ width: "100%", height: 40, marginTop: 12, borderRadius: 8, fontSize: 14 }}>
          {t("onboarding:welcome.start")}</button>
      </div>
    </div>
  );
}
