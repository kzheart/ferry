import { CheckSquare, RadioDot } from "./primitives.jsx";

export function FilterPopover({ anchor, onClose, onClear, children, t }) {
  const width = 272;
  const left = anchor
    ? Math.max(8, Math.min(anchor.right - width, window.innerWidth - width - 8))
    : 66;
  const top = anchor ? anchor.bottom + 6 : 190;
  const maxHeight = Math.max(
    200,
    Math.min(430, window.innerHeight - top - 70),
  );
  return (
    <>
      <div
        onClick={onClose}
        style={{ position: "absolute", inset: 0, zIndex: 35 }}
      />
      <div style={{
        position: "absolute",
        left,
        top,
        width,
        zIndex: 36,
        background: "var(--bg)",
        borderRadius: 10,
        boxShadow: "var(--shadow-menu)",
        overflow: "hidden",
      }}>
        <div className="fscroll" style={{
          maxHeight,
          overflowY: "auto",
          padding: "12px 13px",
        }}>
          {children}
        </div>
        <div style={{
          display: "flex",
          alignItems: "center",
          padding: "9px 13px",
          borderTop: "1px solid var(--line5)",
        }}>
          <a onClick={onClear} style={{ fontSize: 11, color: "var(--tx3b)" }}>
            {t("overlays:filter.clear")}
          </a>
          <span style={{ flex: 1 }} />
          <button
            className="fbtn-primary"
            style={{ height: 28, padding: "0 14px", fontSize: 12 }}
            onClick={onClose}
          >
            {t("overlays:filter.done")}
          </button>
        </div>
      </div>
    </>
  );
}

export function FilterSectionTitle({ children, first }) {
  return (
    <div style={{
      fontSize: 11,
      fontWeight: 600,
      color: "var(--tx5)",
      letterSpacing: ".03em",
      margin: first ? "0 0 6px" : "12px 0 6px",
    }}>
      {children}
    </div>
  );
}

export function FilterCheckRow({ on, onClick, icon, label, extra }) {
  return (
    <div
      className="hov-item"
      onClick={onClick}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 9,
        padding: "6px 7px",
        borderRadius: 6,
        cursor: "default",
      }}
    >
      <CheckSquare on={on} />
      {icon}
      <span style={{
        fontSize: 12,
        color: "var(--tx2)",
        flex: 1,
        whiteSpace: "nowrap",
        overflow: "hidden",
        textOverflow: "ellipsis",
      }}>
        {label}
      </span>
      {extra && (
        <span style={{ fontSize: 11, color: "var(--tx5)" }}>{extra}</span>
      )}
    </div>
  );
}

export function FilterRadioRow({ on, onClick, label }) {
  return (
    <div
      className="hov-item"
      onClick={onClick}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 9,
        padding: "6px 7px",
        borderRadius: 6,
        cursor: "default",
      }}
    >
      <RadioDot on={on} />
      <span style={{
        fontSize: 12,
        color: "var(--tx2)",
        whiteSpace: "nowrap",
        overflow: "hidden",
        textOverflow: "ellipsis",
      }}>
        {label}
      </span>
    </div>
  );
}
