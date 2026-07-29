import { useEffect, useState } from "react";

import { CheckIcon, CloseIcon, Spinner, WarnIcon } from "./icons.jsx";

// 自动消隐:成功提示说完就该让路;带撤销的多留一会儿,别让人来不及点。
// 失败要留给用户读错误,进行中由业务态驱动,两者都不自动消失。
const DISMISS_MS = 3200;
const DISMISS_MS_ACTION = 7000;

// 状态只由这枚 16px 图标承担。整块染色是 Ferry 里唯一的一处,
// 而这套配色的强调色本就是中性黑白——浮层该和右键菜单同材质。
function StatusIcon({ kind }) {
  if (kind === "run") return <Spinner size={16} />;
  return (
    <span style={{ display: "inline-flex", flex: "none",
      color: kind === "ok" ? "var(--ok)" : "var(--err)" }}>
      {kind === "ok" ? <CheckIcon size={16} /> : <WarnIcon size={16} />}
    </span>
  );
}

export function Toast({ toast, onDismiss }) {
  const kind = toast.kind;
  const [hovered, setHovered] = useState(false);
  // 换一条提示要让倒计时细线重新跑满,而组件本身不重挂载
  const [gen, setGen] = useState(0);
  useEffect(() => { setGen(g => g + 1); }, [toast]);

  const autoMs = kind === "ok" ? (toast.action ? DISMISS_MS_ACTION : DISMISS_MS) : 0;
  useEffect(() => {
    if (!autoMs || hovered) return;
    const timer = setTimeout(onDismiss, autoMs);
    return () => clearTimeout(timer);
    // toast 每次都是新对象,换一条提示即重新计时;悬停期间暂停,移开重新计时
  }, [toast, autoMs, hovered, onDismiss]);

  return (
    <div
      className="ftoast"
      role="status"
      aria-live={kind === "fail" ? "assertive" : "polite"}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        position: "absolute",
        left: "50%",
        bottom: 26,
        transform: "translateX(-50%)",
        zIndex: 45,
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "11px 11px 11px 13px",
        borderRadius: 10,
        background: "var(--surface)",
        // 失败比成功重一档,但靠一圈细描边,不靠铺色
        boxShadow: kind === "fail"
          ? "var(--shadow-menu), 0 0 0 1px var(--err-line)"
          : "var(--shadow-menu)",
        maxWidth: 560,
        overflow: "hidden",
      }}
    >
      <StatusIcon kind={kind} />
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: "var(--tx1)",
          letterSpacing: "-.004em" }}>
          {toast.title}
        </div>
        <div style={{ fontSize: 11.5, color: "var(--tx3b)", marginTop: 1, lineHeight: 1.45 }}>
          {toast.desc}
        </div>
      </div>
      {toast.action && (
        <button
          className="fbtn"
          style={{ height: 26, padding: "0 11px", fontSize: 12, flex: "none", fontWeight: 600 }}
          onClick={toast.action.onClick}
        >
          {toast.action.label}
        </button>
      )}
      <button
        className="row-act-btn"
        onClick={onDismiss}
        aria-label={toast.dismissLabel || "Dismiss"}
      >
        <CloseIcon size={12} />
      </button>
      {autoMs > 0 && (
        // 悬停时停成满格而不是冻在半路:移开确实是重新计时,细线得说实话
        <span key={`${gen}-${hovered}`} className="ftoast-bar"
          style={hovered ? undefined : { animation: `ftoast-drain ${autoMs}ms linear forwards` }} />
      )}
    </div>
  );
}
