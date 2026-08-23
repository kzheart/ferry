// 状态即按钮:同一个控件承担「当前状态」和「下一步动作」两件事。
// 静止时看起来是状态标签(圆点 + 文案),指上去才换成动作文案并显形为按钮,
// 点下去变 spinner,跑完圆点做一次 settle 落定成新状态。
//
// 这么做是为了让设置页每行只剩一个控件:原来「状态小字 + 主按钮 + 次按钮」三件
// 东西在说同一件事,信息重复,视觉也吵。次要动作(比如可更新时的「移除」)交给
// 行 hover 时才出现的幽灵按钮,静止时不占位。
import { useRef, useState } from "react";
import { Spinner } from "./icons.jsx";

const TONE = { ok: "var(--ok)", warn: "var(--warn)", err: "var(--err)", idle: "var(--tx5)" };

// spinner 延迟出现:安装本地符号链接这类操作往往几十毫秒就结束,
// 立刻渲染 spinner 只会闪一下,反而像出错了。
const SPINNER_DELAY = 180;

export default function StateButton({
  tone = "idle", stateLabel, actionLabel, pendingLabel, failLabel,
  onRun, danger, disabled, width = 96, title,
}) {
  const [phase, setPhase] = useState("idle");
  const [armed, setArmed] = useState(false);
  const [spinning, setSpinning] = useState(false);
  const [settle, setSettle] = useState(0);
  const busy = useRef(false);

  const run = async () => {
    if (busy.current || disabled) return;
    busy.current = true;
    setPhase("pending");
    const timer = setTimeout(() => setSpinning(true), SPINNER_DELAY);
    try {
      await onRun?.();
      setSettle(n => n + 1);
      setPhase("idle");
    } catch {
      // 失败原因由调用方自己展示(它才知道该说什么);这里只负责把按钮变成「重试」
      setPhase("failed");
    } finally {
      clearTimeout(timer);
      setSpinning(false);
      busy.current = false;
    }
  };

  const pending = phase === "pending";
  const failed = phase === "failed";
  // 禁用态不该显形为动作:hover 也只显示状态,免得点了没反应
  const showAction = armed && !pending && !disabled;
  const label = pending ? (pendingLabel || actionLabel)
    : failed ? (failLabel || actionLabel)
      : showAction ? actionLabel : stateLabel;

  const skin = failed ? { background: "var(--err-bg)", border: "1px solid var(--err-line)", color: "var(--err-deep)" }
    : showAction && danger ? { background: "var(--err-bg)", border: "1px solid var(--err-line)", color: "var(--err-text)" }
      : showAction ? { background: "var(--accent)", border: "1px solid transparent", color: "var(--accent-fg)" }
        : { background: "transparent", border: "1px solid var(--line6)", color: "var(--tx3b)" };

  return (
    <button type="button" onClick={run} disabled={disabled || pending}
      title={title || (disabled ? stateLabel : `${stateLabel} — ${actionLabel}`)}
      onMouseEnter={() => setArmed(true)} onMouseLeave={() => setArmed(false)}
      onFocus={() => setArmed(true)} onBlur={() => setArmed(false)}
      style={{ width, height: 30, flex: "none", borderRadius: 8, padding: "0 9px",
        display: "inline-flex", alignItems: "center", justifyContent: "center", gap: 6,
        fontSize: 12, fontWeight: 600, fontFamily: "inherit", cursor: "default",
        opacity: disabled ? 0.55 : 1, overflow: "hidden",
        transition: "background .14s ease, color .14s ease, border-color .14s ease",
        ...skin }}>
      {spinning
        ? <Spinner size={12} accent={showAction ? "var(--accent-fg)" : "var(--accent)"} />
        : (
          <span key={settle} style={{ width: 6, height: 6, borderRadius: "50%", flex: "none",
            background: showAction || failed ? "currentColor" : TONE[tone] || TONE.idle,
            animation: settle ? "fsettle .34s cubic-bezier(.34,1.4,.64,1)" : undefined }} />
        )}
      <span style={{ whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
        {label}
      </span>
    </button>
  );
}
