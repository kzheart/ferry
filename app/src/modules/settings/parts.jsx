// 设置面板的通用排版件:分组标题 / 卡片 / 行 / 下拉 / 开关
export const GroupTitle = ({ children, first, icon, right }) => (
  <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx5)", letterSpacing: ".05em",
    margin: first ? "0 0 9px 2px" : "22px 0 9px 2px",
    display: "flex", alignItems: "center", gap: 7 }}>
    {icon}{children}
    {right && <span style={{ marginLeft: "auto", fontWeight: 600, letterSpacing: 0,
      color: "var(--tx4)" }}>{right}</span>}
  </div>
);

export const Card = ({ children }) => (
  <div style={{ border: "1px solid var(--line4)", borderRadius: 12, background: "var(--surface)",
    overflow: "hidden" }}>{children}</div>
);

export function Row({ title, desc, children, first, className }) {
  return (
    <div className={className} style={{ display: "flex", alignItems: "center", gap: 12,
      padding: "calc(var(--fs-body) + 1px) 16px",
      borderTop: first ? "none" : "1px solid var(--line6)" }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: "var(--fs-body)", fontWeight: 600, color: "var(--tx1)" }}>{title}</div>
        {desc && <div style={{ fontSize: "var(--fs-meta)", color: "var(--tx4)", marginTop: 2 }}>{desc}</div>}
      </div>
      {children}
    </div>
  );
}

// 原生 select:自带键盘导航与系统弹层,选项多了也不会撑爆设置面板
export function Select({ value, onChange, children, disabled, width }) {
  return (
    <div style={{ position: "relative", flex: "none", width, maxWidth: "100%" }}>
      <select value={value} disabled={disabled} onChange={e => onChange(e.target.value)}
        style={{ appearance: "none", height: 30, padding: "0 28px 0 11px", borderRadius: 8,
          border: "1px solid var(--line4)", color: "var(--tx1)", width: width ? "100%" : undefined,
          background: disabled ? "var(--fill3)" : "var(--surface)",
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

// 勾选框:选中填 accent,对勾用 accent-fg——深色主题 accent 接近白色,写死 #fff 会看不见
export function Check({ on, size = 16 }) {
  return (
    <span style={{ width: size, height: size, borderRadius: 5, flex: "none",
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      border: `1.5px solid ${on ? "var(--accent)" : "var(--tx5)"}`,
      background: on ? "var(--accent)" : "transparent" }}>
      {on && (
        <svg width={size * 0.62} height={size * 0.62} viewBox="0 0 10 10" aria-hidden>
          <path d="M1.6 5.2l2.2 2.2 4.6-4.8" fill="none" stroke="var(--accent-fg)" strokeWidth="1.9"
            strokeLinecap="round" strokeLinejoin="round" />
        </svg>)}
    </span>
  );
}

export const inputStyle = {
  height: 32, border: "1px solid var(--line4)", borderRadius: 8, padding: "0 11px",
  fontSize: 12.5, background: "var(--surface)", color: "var(--tx1)", fontFamily: "inherit",
  outline: "none",
};

// 分段控件:选项少(2–3 个)且需要一眼看全时用它,比下拉少一次点击
export function Segmented({ value, options, onChange, label }) {
  return (
    <div role="radiogroup" aria-label={label}
      style={{ display: "flex", flex: "none", padding: 2, gap: 2, borderRadius: 9,
        background: "var(--fill4)", border: "1px solid var(--line4)" }}>
      {options.map(([key, text]) => {
        const on = key === value;
        return (
          <button key={key} type="button" role="radio" aria-checked={on}
            onClick={() => onChange(key)}
            style={{ height: 26, padding: "0 13px", borderRadius: 7, border: "none",
              background: on ? "var(--seg-on)" : "transparent",
              color: on ? "var(--tx1)" : "var(--tx3)", fontSize: 12, fontWeight: 600,
              fontFamily: "inherit", cursor: "default",
              boxShadow: on ? "0 1px 2px rgba(0,0,0,.08)" : "none",
              transition: "background .12s ease" }}>{text}</button>
        );
      })}
    </div>
  );
}
