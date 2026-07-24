export function ContextMenu({ x, y, items, onClose }) {
  const width = 208;
  const height = items.reduce(
    (total, item) => total + (item.sep ? 9 : 30),
    12,
  );
  const left = Math.max(8, Math.min(x, window.innerWidth - width - 8));
  const top = Math.max(8, Math.min(y, window.innerHeight - height - 8));
  return (
    <>
      <div
        onClick={onClose}
        onContextMenu={event => {
          event.preventDefault();
          onClose();
        }}
        style={{ position: "absolute", inset: 0, zIndex: 55 }}
      />
      <div style={{
        position: "absolute",
        left,
        top,
        width,
        zIndex: 56,
        padding: 6,
        background: "var(--bg)",
        borderRadius: 10,
        boxShadow: "var(--shadow-menu)",
      }}>
        {items.map((item, index) => item.sep ? (
          <div
            key={index}
            style={{
              height: 1,
              background: "var(--line3)",
              margin: "4px 8px",
            }}
          />
        ) : (
          <div
            key={index}
            className={item.disabled ? undefined : "hov-item"}
            onClick={() => {
              if (item.disabled) return;
              onClose();
              item.onClick?.();
            }}
            title={item.disabled ? item.disabledHint : undefined}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              height: 30,
              padding: "0 9px",
              borderRadius: 6,
              fontSize: 12,
              color: item.disabled
                ? "var(--tx5)"
                : item.danger
                  ? "var(--err-text)"
                  : "var(--tx2)",
              cursor: item.disabled ? "default" : "pointer",
              whiteSpace: "nowrap",
            }}
          >
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>
              {item.label}
            </span>
            {item.hint && (
              <span style={{ fontSize: 11, color: "var(--tx5)", flex: "none" }}>
                {item.hint}
              </span>
            )}
          </div>
        ))}
      </div>
    </>
  );
}
