export function ConfirmBox({ width = 400, title, children, actions }) {
  return (
    <div style={{
      position: "absolute",
      inset: 0,
      background: "var(--scrim)",
      display: "flex",
      alignItems: "center",
      justifyContent: "center",
      zIndex: 44,
    }}>
      <div style={{
        width,
        background: "var(--bg)",
        borderRadius: 12,
        boxShadow: "var(--shadow-sheet)",
        padding: 22,
      }}>
        <div style={{ fontSize: 15, fontWeight: 650 }}>{title}</div>
        {children}
        <div style={{
          display: "flex",
          justifyContent: "flex-end",
          gap: 10,
          marginTop: 18,
        }}>
          {actions}
        </div>
      </div>
    </div>
  );
}
