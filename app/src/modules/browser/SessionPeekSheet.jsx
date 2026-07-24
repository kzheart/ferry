import { useTranslation } from "react-i18next";
import { Sheet } from "../../shared/ui/primitives.jsx";
import SessionDetail from "./SessionDetail.jsx";

export function SessionPeekSheet({
  selectedId,
  meta,
  detail,
  actions,
  scope,
  ops,
  dirtyOps,
  applying,
  navigationTarget,
  refreshing,
  onClose,
  onOpenLibrary,
}) {
  const { t } = useTranslation();
  return (
    <Sheet width="min(940px, 94vw)" maxHeight="90vh" onClose={onClose}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "9px 12px 9px 16px",
          borderBottom: "1px solid var(--line5)",
        }}
      >
        <span
          style={{
            flex: 1,
            minWidth: 0,
            fontSize: 13,
            fontWeight: 600,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {meta?.title || meta?.id}
        </span>
        <button
          type="button"
          style={{
            fontSize: 12,
            padding: "4px 10px",
            borderRadius: 7,
            border: "1px solid var(--line3)",
            background: "var(--surface)",
            color: "var(--acc)",
            cursor: "pointer",
          }}
          onClick={onOpenLibrary}
        >
          {t("askferry:peek.openInLibrary")} ↗
        </button>
        <button
          type="button"
          style={{
            fontSize: 12,
            padding: "4px 10px",
            borderRadius: 7,
            border: "1px solid var(--line3)",
            background: "var(--surface)",
            color: "var(--tx3)",
            cursor: "pointer",
          }}
          onClick={onClose}
        >
          {t("askferry:peek.close")}
        </button>
      </div>
      <div
        style={{
          height: "min(720px, 78vh)",
          display: "flex",
          minHeight: 0,
        }}
      >
        <SessionDetail
          key={selectedId}
          meta={meta}
          data={detail?.data}
          error={detail?.error}
          onDiscardAll={actions.onDiscardAll}
          scope={scope}
          setScope={actions.setScope}
          ops={ops}
          dirtyOps={dirtyOps}
          addOp={actions.addOp}
          removeOp={actions.removeOp}
          updateOp={actions.updateOp}
          startReplyEdit={actions.startReplyEdit}
          replyEditError={actions.replyEditError}
          onOpenDiff={actions.onOpenDiff}
          onApply={actions.onApply}
          applying={applying}
          onOpenMigrate={actions.onOpenMigrate}
          navigationTarget={navigationTarget}
          onRefresh={actions.onRefresh}
          refreshing={refreshing}
          onResume={actions.onResume}
        />
      </div>
    </Sheet>
  );
}
