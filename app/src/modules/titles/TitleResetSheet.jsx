// AI 重置标题的预览面板:旧标题 → 新标题(可改)、类型、跳过原因、逐条勾选。
// 生成期间只有一个等待态;模型没配好时把用户指到「设置 → 模型」,不假装能生成。
import { useTranslation } from "react-i18next";

import { Sheet, CheckSquare } from "../../shared/ui/primitives.jsx";
import { Spinner, ToolIcon } from "../../shared/ui/icons.jsx";
import { useTitleReset } from "./useTitleReset.js";

const REASON_KEYS = new Set(["manual", "too_short", "too_long", "duplicate", "error"]);

function ReasonChip({ reason, detail, t }) {
  const key = REASON_KEYS.has(reason) ? reason : "unknown";
  return (
    <span style={{ flex: "none", padding: "3px 9px", borderRadius: 20, fontSize: 11,
      background: "var(--fill4)", color: "var(--tx4)" }}>
      {t(`overlays:titleReset.reason.${key}`)}{detail ? ` · ${detail}` : ""}
    </span>
  );
}

function TitleRow({ row, onChange, t }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "10px 14px",
      borderTop: "1px solid var(--line6)", opacity: row.skip ? 0.55 : 1 }}>
      <button type="button" aria-label={row.before || row.ref}
        disabled={row.skip} onClick={() => onChange({ checked: !row.checked })}
        style={{ border: "none", background: "transparent", padding: 0, cursor: "default",
          display: "inline-flex", flex: "none" }}>
        <CheckSquare on={row.checked} />
      </button>
      <ToolIcon tool={row.tool} size={18} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 11.5, color: "var(--tx4)", whiteSpace: "nowrap",
          overflow: "hidden", textOverflow: "ellipsis" }}>
          {row.before || t("overlays:titleReset.noTitle")}
        </div>
        {row.skip ? (
          <div style={{ fontSize: 12.5, color: "var(--tx3b)", marginTop: 3 }}>
            {t("overlays:titleReset.skipped")}
          </div>
        ) : (
          <input
            value={row.after}
            aria-label={t("overlays:titleReset.after")}
            onChange={event => onChange({ after: event.target.value })}
            style={{ width: "100%", boxSizing: "border-box", height: 30, marginTop: 3,
              padding: "0 10px", background: "var(--surface)", border: "1px solid var(--line4)",
              borderRadius: 8, fontSize: 12.5, color: "var(--tx1)", fontFamily: "inherit",
              outline: "none" }}
          />
        )}
      </div>
      {row.skip
        ? <ReasonChip reason={row.reason} detail={row.detail} t={t} />
        : row.type && (
          <span style={{ flex: "none", fontSize: 13 }} title={t("overlays:titleReset.type")}>
            {row.type}
          </span>
        )}
    </div>
  );
}

export default function TitleResetSheet({
  sessions, batch, renameSession, updateMetadata, rescan, setToast, onClose, onOpenModels,
}) {
  const { t } = useTranslation();
  const reset = useTitleReset({
    sessions, batch, renameSession, updateMetadata, rescan, setToast, onClose, t,
  });
  const busy = reset.status === "loading" || reset.status === "applying";
  const providerMissing = reset.error?.code === "provider_unavailable";

  let body = null;
  if (reset.status === "loading" || reset.status === "applying") {
    body = (
      <div style={{ padding: "70px 0", display: "flex", alignItems: "center",
        justifyContent: "center", gap: 10, color: "var(--tx4)", fontSize: 13 }}>
        <Spinner size={16} />
        {t(reset.status === "applying"
          ? "overlays:titleReset.applying"
          : "overlays:titleReset.generating")}
      </div>
    );
  } else if (reset.status === "error") {
    body = (
      <div style={{ border: "1px solid var(--err-line)", background: "var(--err-bg)",
        borderRadius: 10, padding: "16px 18px" }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: "var(--err-text)" }}>
          {t(providerMissing
            ? "overlays:titleReset.providerUnavailable"
            : "overlays:titleReset.failed")}
        </div>
        <div style={{ fontSize: 12, color: "var(--err-mut)", marginTop: 6, lineHeight: 1.55 }}>
          {providerMissing ? t("overlays:titleReset.providerHint") : reset.error?.message}
        </div>
        <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
          {providerMissing && onOpenModels && (
            <button className="fbtn-primary" style={{ height: 30, padding: "0 13px", fontSize: 12 }}
              onClick={onOpenModels}>{t("overlays:titleReset.openModels")}</button>
          )}
          <button className="fbtn" style={{ height: 30, fontSize: 12 }} onClick={reset.retry}>
            {t("overlays:titleReset.retry")}
          </button>
        </div>
      </div>
    );
  } else if (!reset.rows.length) {
    body = (
      <div style={{ padding: "70px 0", textAlign: "center", color: "var(--tx4)", fontSize: 13 }}>
        {t("overlays:titleReset.empty")}
      </div>
    );
  } else {
    body = (
      <div className="fcard" style={{ overflow: "hidden" }}>
        {reset.rows.map(row => (
          <TitleRow key={row.key} row={row} t={t}
            onChange={patch => reset.updateRow(row.key, patch)} />
        ))}
      </div>
    );
  }

  return (
    <Sheet width={720} maxHeight={760} onClose={busy ? undefined : onClose}>
      <div style={{ flex: "none", padding: "15px 20px", borderBottom: "1px solid var(--line5)",
        display: "flex", alignItems: "center", gap: 12 }}>
        <div style={{ fontSize: 14, fontWeight: 600 }}>
          {batch
            ? t("overlays:titleReset.batchTitle", { n: sessions.length })
            : t("overlays:titleReset.title")}
        </div>
        <div style={{ fontSize: 11.5, color: "var(--tx4)" }}>
          {reset.model
            ? t("overlays:titleReset.model", { model: reset.model.model })
            : t("overlays:titleReset.modelUnknown")}
        </div>
        <div style={{ flex: 1 }} />
        {!busy && (
          <a onClick={onClose} style={{ color: "var(--tx5)", fontSize: 18, lineHeight: 1 }}>×</a>
        )}
      </div>
      <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: 20 }}>{body}</div>
      <div style={{ flex: "none", padding: "13px 20px", borderTop: "1px solid var(--line5)",
        display: "flex", alignItems: "center", gap: 10 }}>
        <button type="button" className="fbtn" disabled={busy}
          onClick={() => reset.setIncludeManual(!reset.includeManual)}
          style={{ height: 30, fontSize: 12, display: "inline-flex", alignItems: "center", gap: 8 }}>
          <CheckSquare on={reset.includeManual} size={13} />
          {t("overlays:titleReset.includeManual")}
        </button>
        <div style={{ flex: 1 }} />
        <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onClose}
          disabled={reset.status === "applying"}>
          {t("overlays:titleReset.cancel")}
        </button>
        <button className="fbtn-primary" style={{ height: 34, padding: "0 18px", fontSize: 13 }}
          disabled={busy || !reset.selectedCount} onClick={reset.apply}>
          {t("overlays:titleReset.apply", { n: reset.selectedCount })}
        </button>
      </div>
    </Sheet>
  );
}
