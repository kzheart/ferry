import { useTranslation } from "react-i18next";

import { renderEvents } from "../../shared/contracts/events.js";
import { ConfirmBox } from "../../shared/ui/ConfirmBox.jsx";
import { Spinner } from "../../shared/ui/icons.jsx";
import { Sheet } from "../../shared/ui/primitives.jsx";
import { fmtSize } from "../../shared/ui/toolDisplay.js";

export function DiffSheet({ ops, preview, loading, error, onClose }) {
  const { t } = useTranslation();
  const replyText = items => {
    const limit = 8000;
    let text = "";
    for (const item of items || []) {
      const input = typeof item.input === "string"
        ? item.input
        : JSON.stringify(item.input, null, 2);
      const part = item.kind === "text"
        ? `${t("overlays:diff.replyTextLabel")}\n${item.text}`
        : `${t("overlays:diff.replyToolLabel", { name: item.name })}\n`
          + `${t("overlays:diff.replyParamsLabel")} ${input}\n`
          + `${t("overlays:diff.replyOutputLabel")}\n${item.output}`;
      const room = limit - text.length;
      if (room <= 0) break;
      text += (text ? "\n\n" : "") + part.slice(0, room);
    }
    return text.length >= limit
      ? `${text.slice(0, limit)}\n${t("overlays:diff.previewTruncated")}`
      : text;
  };

  return (
    <Sheet width={760} maxHeight={780} onClose={onClose}>
      <div style={{
        flex: "none",
        padding: "15px 20px",
        borderBottom: "1px solid var(--line5)",
        display: "flex",
        alignItems: "center",
      }}>
        <div style={{ fontSize: 14, fontWeight: 650 }}>
          {t("overlays:diff.title")}
        </div>
        <div style={{ fontSize: 12, color: "var(--tx4)", marginLeft: 12 }}>
          {t("overlays:diff.metaOps", { n: ops.length })}
          {preview && `${t("overlays:diff.metaSize", {
            before: fmtSize(preview.before.size),
            after: fmtSize(preview.after.size),
          })}
            ${t("overlays:diff.metaCount", {
              before: preview.before.count,
              after: preview.after.count,
            })}`}
        </div>
        <div style={{ flex: 1 }} />
        <a onClick={onClose} style={{ color: "var(--tx5)", fontSize: 18 }}>×</a>
      </div>
      <div className="fscroll" style={{
        flex: 1,
        overflowY: "auto",
        padding: "18px 20px",
      }}>
        {ops.length === 0 && (
          <div style={{
            textAlign: "center",
            color: "var(--tx5)",
            fontSize: 13,
            padding: 40,
          }}>
            {t("overlays:diff.empty")}
          </div>
        )}
        {loading && (
          <div style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            color: "var(--tx4)",
            fontSize: 12,
            marginBottom: 14,
          }}>
            <Spinner size={14} /> {t("overlays:diff.loading")}
          </div>
        )}
        {error && (
          <div style={{
            padding: "9px 12px",
            borderRadius: 8,
            background: "var(--err-bg2)",
            color: "var(--err-text)",
            fontSize: 12,
            marginBottom: 12,
          }}>
            {error}
          </div>
        )}
        {ops.map(operation => (
          <div
            key={operation.id}
            style={{
              border: "1px solid var(--line3)",
              borderRadius: 10,
              overflow: "hidden",
              marginBottom: 12,
            }}
          >
            <div style={{
              padding: "9px 13px",
              background: "var(--fill2)",
              borderBottom: "1px solid var(--line5)",
              display: "flex",
              alignItems: "center",
              gap: 9,
            }}>
              <span style={{
                width: 7,
                height: 7,
                borderRadius: "50%",
                background: operation.dot,
              }} />
              <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2)" }}>
                {operation.labelKey
                  ? t(operation.labelKey, operation.labelParams)
                  : operation.label}
              </span>
              {operation.type === "rewrite"
                && operation.text === operation.orig && (
                  <span style={{
                    fontSize: 11,
                    color: "var(--warn-deep)",
                    marginLeft: "auto",
                  }}>
                    {t("overlays:diff.contentUnchanged")}
                  </span>
                )}
            </div>
            <div className="mono selectable" style={{
              padding: "11px 13px",
              fontSize: 11,
              lineHeight: 1.7,
            }}>
              <div className="fscroll" style={{
                background: "var(--err-bg2)",
                color: "var(--err-text)",
                padding: "6px 10px",
                borderRadius: 5,
                whiteSpace: "pre-wrap",
                overflowWrap: "break-word",
                maxHeight: 180,
                overflowY: "auto",
              }}>
                − {operation.type === "assistant-reply"
                  ? replyText(operation.origItems).slice(0, 8000)
                  : (operation.orig || t("overlays:diff.noUserMessage"))
                    .slice(0, 4000)}
              </div>
              {operation.type === "delete" ? (
                <div style={{ fontSize: 11, color: "var(--tx5)", marginTop: 6 }}>
                  {operation.summary}
                </div>
              ) : (
                <div className="fscroll" style={{
                  background: "var(--ok-bg2)",
                  color: "var(--ok-body2)",
                  padding: "6px 10px",
                  borderRadius: 5,
                  marginTop: 5,
                  whiteSpace: "pre-wrap",
                  overflowWrap: "break-word",
                  maxHeight: 180,
                  overflowY: "auto",
                }}>
                  + {operation.type === "assistant-reply"
                    ? replyText(operation.items.map(item => item.kind === "tool"
                      ? { ...item, input: item.inputText }
                      : item)).slice(0, 8000)
                    : (operation.text || "").slice(0, 4000)}
                </div>
              )}
            </div>
          </div>
        ))}
        {preview?.changes?.length > 0 && (
          <div style={{ fontSize: 12, color: "var(--tx3b)", lineHeight: 1.6 }}>
            {t("overlays:diff.engineConfirm", {
              changes: renderEvents(preview.changes).join(";"),
            })}
          </div>
        )}
      </div>
      <div style={{
        flex: "none",
        padding: "13px 20px",
        borderTop: "1px solid var(--line5)",
        display: "flex",
        justifyContent: "flex-end",
      }}>
        <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onClose}>
          {t("overlays:diff.close")}
        </button>
      </div>
    </Sheet>
  );
}

export function ApplyConfirm({ ops, onCancel, onConfirm }) {
  const { t } = useTranslation();
  return (
    <ConfirmBox
      width={440}
      title={t("overlays:apply.title", { n: ops.length })}
      actions={(
        <>
          <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>
            {t("overlays:apply.cancel")}
          </button>
          <button
            style={{
              height: 34,
              padding: "0 16px",
              background: "var(--err2)",
              border: "none",
              borderRadius: 8,
              fontSize: 13,
              color: "#fff",
              cursor: "default",
              fontWeight: 600,
            }}
            onClick={onConfirm}
          >
            {t("overlays:apply.confirmInplace")}
          </button>
        </>
      )}
    >
      <div style={{
        fontSize: 12,
        color: "var(--tx3b)",
        marginTop: 12,
        lineHeight: 1.55,
      }}>
        {t("overlays:apply.inplaceFootnote")}
      </div>
    </ConfirmBox>
  );
}
