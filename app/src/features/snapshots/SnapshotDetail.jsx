// 快照详情:状态/大小/创建原因/关联会话/变更摘要/差异预览 + 还原入口
import { useTranslation } from "react-i18next";
import { TOOL_NAME } from "../../api/contract/tools.js";
import { fmtSize } from "../../domain/tools/toolDisplay.js";
import { fmtTime } from "../../domain/sessions/sessionModel.js";
import { ToolIcon } from "../../components/ui/icons.jsx";

const REASON_TEXT_KEY = {
  "会话编辑前自动": "snapshots:reasonText.beforeEdit",
  "还原前保护": "snapshots:reasonText.beforeRestoreGuard",
  "迁移前自动": "snapshots:reasonText.beforeMigrate",
};

export default function SnapshotDetail({ s, restoring, onRestore }) {
  const { t } = useTranslation();
  if (!s) return (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
      color: "var(--tx5)", fontSize: 13 }}>{t("snapshots:empty")}</div>
  );
  const state = restoring === "done" ? t("snapshots:state.restored")
    : restoring ? t("snapshots:state.restoring") : t("snapshots:state.restorable");
  const stColor = restoring ? "var(--warn)" : "var(--ok)";
  const busy = restoring === true;
  const done = restoring === "done";
  const btnLabel = busy ? t("snapshots:btn.restoring")
    : done ? t("snapshots:btn.restored") : t("snapshots:btn.restore");
  const reasonTextKey = REASON_TEXT_KEY[s.trigger];
  return (
    <div className="fscroll" style={{ flex: 1, overflowY: "auto", minHeight: 0, animation: "ffade .16s ease" }}>
      <div style={{ padding: "20px 26px 16px", borderBottom: "1px solid var(--line5)" }}>
        <div style={{ display: "flex", alignItems: "flex-start", gap: 13 }}>
          <ToolIcon tool={s.tool || "claude"} size={40} dot={stColor} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 16, fontWeight: 650, letterSpacing: "-.01em" }}>{s.title}</div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "6px 14px", marginTop: 6,
              fontSize: 12, color: "var(--tx3b)" }}>
              <span className="mono" style={{ color: "var(--tx4)" }}>{s.id}</span>
              <span>{s.trigger}</span>
              <span>{fmtTime(s.time, t)}</span>
              <span>{fmtSize(s.size)}</span>
            </div>
          </div>
          <button onClick={busy || done ? undefined : onRestore}
            style={{ height: 32, padding: "0 15px", background: busy || done ? "var(--fill4)" : "var(--accent)",
              border: "none", borderRadius: 8, fontSize: 12.5, color: busy || done ? "var(--tx5)" : "#fff",
              cursor: busy || done ? "default" : "pointer", fontWeight: 600, flex: "none" }}>
            {btnLabel}</button>
        </div>
      </div>
      <div style={{ padding: "20px 26px 44px", maxWidth: 760 }}>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10 }}>
          <div style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "13px 15px" }}>
            <div style={{ fontSize: 11.5, color: "var(--tx4)", fontWeight: 600 }}>{t("snapshots:field.status")}</div>
            <div style={{ display: "inline-flex", alignItems: "center", gap: 6, color: stColor,
              fontWeight: 600, fontSize: 13, marginTop: 7 }}>
              <span style={{ width: 6, height: 6, borderRadius: "50%", background: stColor }} />{state}
            </div>
          </div>
          <div style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "13px 15px" }}>
            <div style={{ fontSize: 11.5, color: "var(--tx4)", fontWeight: 600 }}>{t("snapshots:field.size")}</div>
            <div className="mono" style={{ fontSize: 13, color: "var(--tx2)", marginTop: 7 }}>{fmtSize(s.size)}</div>
          </div>
        </div>

        <div style={{ marginTop: 14, border: "1px solid var(--line3)", borderRadius: 10, padding: "14px 16px" }}>
          <div style={{ fontSize: 11.5, color: "var(--tx4)", fontWeight: 600 }}>{t("snapshots:field.reason")}</div>
          <div style={{ fontSize: 13, color: "var(--tx2)", fontWeight: 600, marginTop: 6 }}>
            {t("snapshots:field.reasonMeta", { trigger: s.trigger, time: fmtTime(s.time, t) })}</div>
          <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 4, lineHeight: 1.55 }}>
            {reasonTextKey ? t(reasonTextKey) : t("snapshots:field.reasonDefault")}</div>
        </div>

        <div style={{ marginTop: 14, border: "1px solid var(--line3)", borderRadius: 10, padding: "13px 15px",
          display: "flex", alignItems: "center", gap: 11 }}>
          <div style={{ fontSize: 11.5, color: "var(--tx4)", fontWeight: 600, flex: "none" }}>{t("snapshots:field.relatedSession")}</div>
          <span style={{ fontSize: 12.5, color: "var(--tx2)", flex: 1, textAlign: "right",
            whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{s.title}</span>
          <span style={{ fontSize: 11.5, color: "var(--tx4)", flex: "none" }}>
            {TOOL_NAME[s.tool] || s.tool}</span>
        </div>

        <div style={{ fontSize: 12, fontWeight: 600, color: "var(--tx3b)", margin: "18px 0 8px" }}>{t("snapshots:summary.title")}</div>
        <div style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "12px 15px" }}>
          {[t("snapshots:summary.fullState"),
            t("snapshots:summary.fileMeta", { name: s.path.split("/").pop(), size: fmtSize(s.size) })].map((c, i) => (
            <div key={i} style={{ display: "flex", alignItems: "flex-start", gap: 8, fontSize: 12.5,
              color: "var(--tx2b)", margin: "5px 0", lineHeight: 1.45 }}>
              <span style={{ width: 5, height: 5, borderRadius: "50%", background: "var(--info-dot)",
                flex: "none", marginTop: 6 }} />{c}
            </div>
          ))}
        </div>

        <div style={{ fontSize: 12, fontWeight: 600, color: "var(--tx3b)", margin: "18px 0 8px" }}>{t("snapshots:diff.title")}</div>
        <div className="mono" style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "12px 14px",
          fontSize: 11.5, lineHeight: 1.7 }}>
          <div style={{ background: "var(--err-bg2)", color: "var(--err-text)", padding: "2px 8px", borderRadius: 5,
            marginBottom: 5 }}>{t("snapshots:diff.currentMinus")}</div>
          <div style={{ background: "var(--ok-bg2)", color: "var(--ok-body2)", padding: "2px 8px", borderRadius: 5 }}>
            {t("snapshots:diff.snapshotPlus", { id: s.id, time: fmtTime(s.time, t) })}</div>
        </div>

        {s.result && !s.result.ok && (
          <div style={{ marginTop: 14, border: "1px solid var(--err-line)", background: "var(--err-bg)",
            borderRadius: 10, padding: "13px 15px", fontSize: 12.5, color: "var(--err-text)", lineHeight: 1.55 }}>
            <div style={{ fontWeight: 600, marginBottom: 4 }}>{t("snapshots:lastRestore")}</div>{s.result.error}
          </div>
        )}

        <div style={{ fontSize: 11.5, color: "var(--tx5)", marginTop: 14, lineHeight: 1.6 }}>
          {t("snapshots:epilogue")}</div>
      </div>
    </div>
  );
}
