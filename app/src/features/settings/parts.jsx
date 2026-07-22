// 设置面板的通用排版件:分组标题 / 卡片 / 行 / 下拉 / 开关
export const GroupTitle = ({ children, first }) => (
  <div style={{ fontSize: 11, fontWeight: 700, color: "var(--tx5)", letterSpacing: ".05em",
    margin: first ? "0 0 9px 2px" : "22px 0 9px 2px" }}>{children}</div>
);

export const Card = ({ children }) => (
  <div style={{ border: "1px solid var(--line4)", borderRadius: 12, background: "var(--surface)",
    overflow: "hidden" }}>{children}</div>
);

export function Row({ title, desc, children, first }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 12, padding: "14px 16px",
      borderTop: first ? "none" : "1px solid var(--line6)" }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: "var(--tx1)" }}>{title}</div>
        {desc && <div style={{ fontSize: 11, color: "var(--tx4)", marginTop: 2 }}>{desc}</div>}
      </div>
      {children}
    </div>
  );
}

// 原生 select:自带键盘导航与系统弹层,选项多了也不会撑爆设置面板
export function Select({ value, onChange, children }) {
  return (
    <div style={{ position: "relative", flex: "none" }}>
      <select value={value} onChange={e => onChange(e.target.value)}
        style={{ appearance: "none", height: 30, padding: "0 28px 0 11px", borderRadius: 8,
          border: "1px solid var(--line4)", background: "var(--surface)", color: "var(--tx1)",
          fontSize: 12, fontWeight: 600, fontFamily: "inherit", cursor: "default" }}>
        {children}
      </select>
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden
        style={{ position: "absolute", right: 10, top: "50%", transform: "translateY(-50%)",
          pointerEvents: "none", color: "var(--tx4)" }}>
        <path d="M2 4l3 3 3-3" fill="none" stroke="currentColor" strokeWidth="1.6"
          strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    </div>
  );
}

export function Toggle({ on, onChange, size = 26 }) {
  const knob = size - 6;
  return (
    <button onClick={() => onChange(!on)} aria-pressed={on}
      style={{ width: size * 1.7, height: size, borderRadius: 20, border: "none", flex: "none",
        background: on ? "var(--accent)" : "var(--toggle-off)", cursor: "default", padding: 0,
        position: "relative", transition: "background .15s ease" }}>
      <span style={{ position: "absolute", top: 3, left: on ? size * 1.7 - knob - 3 : 3,
        width: knob, height: knob, borderRadius: "50%", background: "var(--surface)",
        boxShadow: "0 1px 3px rgba(0,0,0,.28)", transition: "left .15s ease" }} />
    </button>
  );
}

export const inputStyle = {
  height: 32, border: "1px solid var(--line4)", borderRadius: 8, padding: "0 11px",
  fontSize: 12.5, background: "var(--surface)", color: "var(--tx1)", fontFamily: "inherit",
  outline: "none",
};
