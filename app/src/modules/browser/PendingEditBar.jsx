import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Caret, CloseIcon } from "../../shared/ui/icons.jsx";

export default function PendingEditBar({
  ops,
  removeOp,
  onOpenDiff,
  onApply,
  applying,
  invalid,
  onDiscardAll,
}) {
  const { t: tt } = useTranslation();
  const [listOpen, setListOpen] = useState(false);
  const jump = round => {
    document.querySelector(`[data-round="${round}"]`)
      ?.scrollIntoView({
        behavior: "smooth",
        block: "center",
      });
  };

  return (
    <div
      style={{
        position: "absolute",
        bottom: 20,
        left: "50%",
        transform: "translateX(-50%)",
        zIndex: 5,
      }}
    >
      {listOpen && (
        <div
          className="fscroll"
          style={{
            position: "absolute",
            bottom: "calc(100% + 8px)",
            left: 0,
            minWidth: 250,
            maxHeight: 262,
            overflowY: "auto",
            background: "var(--bg)",
            border: "1px solid var(--line3)",
            borderRadius: 10,
            boxShadow: "var(--shadow-menu)",
            padding: 5,
          }}
        >
          {ops.map(operation => (
            <div
              key={operation.id}
              className="hov-ghost"
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "5px 4px 5px 9px",
                borderRadius: 6,
              }}
            >
              <span
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  background: operation.dot,
                  flex: "none",
                }}
              />
              <button
                type="button"
                onClick={() => jump(operation.n)}
                style={{
                  flex: 1,
                  padding: 0,
                  border: 0,
                  background: "transparent",
                  font: "inherit",
                  fontSize: 12,
                  color: "var(--tx2)",
                  cursor: "default",
                  whiteSpace: "nowrap",
                  textAlign: "left",
                }}
              >
                {operation.labelKey
                  ? tt(
                    operation.labelKey,
                    operation.labelParams,
                  )
                  : operation.label}
              </button>
              <button
                type="button"
                className="ficon-btn"
                title={tt("browser:pendingBar.undoOp")}
                onClick={() => removeOp(operation.id)}
              >
                <CloseIcon size={11} />
              </button>
            </div>
          ))}
        </div>
      )}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 7,
          padding: 7,
          background: "var(--bg)",
          border: "1px solid var(--line3)",
          borderRadius: 24,
          boxShadow: "var(--shadow-sheet)",
        }}
      >
        <button
          className="fbtn"
          style={{
            height: 28,
            fontSize: 12,
            borderRadius: 18,
            fontWeight: 600,
          }}
          onClick={() => setListOpen(value => !value)}
        >
          {tt("browser:pendingBar.pendingCount", {
            n: ops.length,
          })}{" "}
          <Caret open={listOpen} size={9} />
        </button>
        <button
          className="fbtn"
          style={{
            height: 28,
            fontSize: 12,
            borderRadius: 18,
          }}
          disabled={!!invalid}
          title={invalid || undefined}
          onClick={onOpenDiff}
        >
          {tt("browser:pendingBar.previewDiff")}
        </button>
        <button
          className="fbtn-primary"
          style={{
            height: 28,
            fontSize: 12,
            padding: "0 14px",
            borderRadius: 18,
          }}
          disabled={applying || !!invalid}
          title={invalid || undefined}
          onClick={onApply}
        >
          {applying
            ? tt("browser:pendingBar.applying")
            : tt("browser:pendingBar.applyChanges")}
        </button>
        <button
          className="fbtn"
          style={{
            height: 28,
            fontSize: 12,
            borderRadius: 18,
            color: "var(--tx4)",
          }}
          onClick={onDiscardAll}
        >
          {tt("browser:pendingBar.discard")}
        </button>
      </div>
      {invalid && (
        <div
          style={{
            position: "absolute",
            right: 14,
            bottom: "calc(100% + 5px)",
            maxWidth: 360,
            padding: "5px 9px",
            borderRadius: 6,
            background: "var(--err-bg2)",
            color: "var(--err-text)",
            fontSize: 11,
          }}
        >
          {invalid}
        </div>
      )}
    </div>
  );
}
