import { Spinner } from "./icons.jsx";

export function Toast({ toast, onDismiss }) {
  const kind = toast.kind;
  const background = kind === "fail"
    ? "var(--err-bg)"
    : kind === "ok"
      ? "var(--ok-bg)"
      : "var(--bg)";
  const border = kind === "fail"
    ? "var(--err-line)"
    : kind === "ok"
      ? "var(--ok-line)"
      : "var(--line3)";
  const color = kind === "fail"
    ? "var(--err-text)"
    : kind === "ok"
      ? "var(--ok-deep)"
      : "var(--tx2)";
  return (
    <div style={{
      position: "absolute",
      left: "50%",
      bottom: 26,
      transform: "translateX(-50%)",
      zIndex: 45,
      display: "flex",
      alignItems: "center",
      gap: 11,
      padding: "12px 16px",
      borderRadius: 10,
      background,
      border: `1px solid ${border}`,
      boxShadow: "var(--shadow-sheet)",
      maxWidth: 560,
    }}>
      {kind === "run" ? (
        <Spinner size={20} track="var(--line)" />
      ) : (
        <span style={{
          width: 26,
          height: 26,
          flex: "none",
          borderRadius: "50%",
          background: kind === "ok" ? "var(--ok)" : "var(--err)",
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          color: "#fff",
          fontSize: 14,
        }}>
          {kind === "ok" ? "✓" : "×"}
        </span>
      )}
      <div>
        <div style={{ fontSize: 13, fontWeight: 600, color }}>
          {toast.title}
        </div>
        <div style={{ fontSize: 11, color: "var(--tx3b)", marginTop: 2 }}>
          {toast.desc}
        </div>
      </div>
      {toast.action && (
        <button
          className="fbtn"
          style={{
            height: 28,
            padding: "0 12px",
            fontSize: 12,
            flex: "none",
            fontWeight: 600,
          }}
          onClick={toast.action.onClick}
        >
          {toast.action.label}
        </button>
      )}
      <a onClick={onDismiss} style={{ color: "var(--tx5)", fontSize: 16, marginLeft: 6 }}>
        ×
      </a>
    </div>
  );
}
