// 会话优化的全部 UI 面:头部分体魔法棒(角色下拉)、Agent 进度细条、
// 内联 diff、底部悬浮汇总栏/多选栏、右侧缩略指示条。
// 状态与编排都在 useSessionOptimization,这里只做展示与转发。
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { roleColorVar } from "../../shared/ui/roleIcons.js";
import {
  Caret,
  CheckIcon,
  CloseIcon,
  RoleAvatar,
  Spinner,
  WandIcon,
} from "../../shared/ui/icons.jsx";

/** 头部分体魔法棒:主按钮直接用绑定角色跑整段,窄箭头弹角色下拉。 */
export function OptimizerWandControl({ optimization, disabled, onStart }) {
  const { t: tt } = useTranslation();
  const [open, setOpen] = useState(false);
  const rootRef = useRef(null);
  const { role, eligibleRoles, setRoleId, status } = optimization;

  useEffect(() => {
    if (!open) return;
    const onDown = event => {
      if (!rootRef.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  if (!role) return null;
  const tint = roleColorVar(role.color);
  const busy = status === "running" || status === "applying";
  return (
    <div ref={rootRef} style={{ display: "flex", alignItems: "center",
      position: "relative", flex: "none" }}>
      <button
        data-optimize="session"
        className="ftool-btn"
        disabled={disabled || busy}
        title={tt("browser:optimize.run", { role: role.name })}
        onClick={() => (onStart ? onStart() : optimization.start())}
        style={{ color: tint, borderRadius: "8px 0 0 8px" }}
      >
        <WandIcon />
      </button>
      <button
        className="ftool-btn"
        disabled={disabled}
        title={tt("browser:optimize.pickRole")}
        onClick={() => setOpen(value => !value)}
        style={{ width: 15, borderRadius: "0 8px 8px 0",
          ...(open ? { background: "var(--hov)" } : {}) }}
      >
        <Caret open={open} size={9} />
      </button>
      {open && (
        <div style={{ position: "absolute", top: 36, right: 0, zIndex: 30,
          width: 280, background: "var(--surface, var(--bg))",
          border: "1px solid var(--line2)", borderRadius: 12,
          boxShadow: "0 16px 44px rgba(0,0,0,.16), 0 3px 10px rgba(0,0,0,.08)",
          padding: 6 }}>
          <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx4)",
            padding: "6px 10px 7px" }}>
            {tt("browser:optimize.pickRoleTitle")}
          </div>
          {eligibleRoles.map(item => (
            <div
              key={item.id}
              onClick={() => { setRoleId(item.id); setOpen(false); }}
              className="hov-ghost"
              style={{ display: "flex", gap: 10, padding: "8px 10px",
                borderRadius: 9, cursor: "default", alignItems: "flex-start" }}
            >
              <RoleAvatar icon={item.icon} color={item.color} size={28} />
              <div style={{ minWidth: 0, flex: 1 }}>
                <div style={{ fontSize: 12.5, fontWeight: 600,
                  display: "flex", alignItems: "center", gap: 6 }}>
                  <span style={{ overflow: "hidden", textOverflow: "ellipsis",
                    whiteSpace: "nowrap" }}>{item.name}</span>
                  {item.id === role.id && (
                    <span style={{ color: "var(--ok)", display: "inline-flex" }}>
                      <CheckIcon size={12} />
                    </span>
                  )}
                </div>
                {item.description && (
                  <div style={{ fontSize: 11, color: "var(--tx4)",
                    marginTop: 1, overflow: "hidden", display: "-webkit-box",
                    WebkitLineClamp: 2, WebkitBoxOrient: "vertical" }}>
                    {item.description}
                  </div>
                )}
                {/* 优化跑在该角色配置的模型上;没配则跟随会话模型 */}
                <div className="mono" style={{ fontSize: 10, color: "var(--tx5)",
                  marginTop: 2, overflow: "hidden", textOverflow: "ellipsis",
                  whiteSpace: "nowrap" }}>
                  {item.model
                    ? item.model.model
                    : tt("browser:optimize.modelFollows")}
                </div>
              </div>
            </div>
          ))}
          <div style={{ borderTop: "1px solid var(--line5)", marginTop: 4,
            padding: "7px 10px 4px", fontSize: 11, color: "var(--tx4)" }}>
            {tt("browser:optimize.pickRoleHint")}
          </div>
        </div>
      )}
    </div>
  );
}

/** Agent 进行中的细条:角色 + 当前工具名 + 停止;跑完自动消失。 */
export function OptimizationAgentBar({ optimization }) {
  const { t: tt } = useTranslation();
  const { status, role, progressTool } = optimization;
  if (status !== "running" || !role) return null;
  const tint = roleColorVar(role.color);
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 9,
      margin: "12px 26px 0", padding: "8px 13px",
      background: `color-mix(in srgb, ${tint} 8%, transparent)`,
      border: `1px solid color-mix(in srgb, ${tint} 25%, transparent)`,
      borderRadius: 10, fontSize: 12.5 }}>
      <RoleAvatar icon={role.icon} color={role.color} size={22} />
      <Spinner size={13} />
      <span style={{ color: "var(--tx2)" }}>
        {tt("browser:optimize.analyzing", { role: role.name })}
      </span>
      {progressTool && (
        <span className="mono" style={{ marginLeft: "auto", fontSize: 11,
          color: "var(--tx4)" }}>{progressTool}</span>
      )}
      <button className="ficon-btn" title={tt("browser:optimize.stop")}
        onClick={optimization.stop}
        style={progressTool ? undefined : { marginLeft: "auto" }}>
        <CloseIcon />
      </button>
    </div>
  );
}

/** 优化出错/无候选的提示条,可关闭。 */
export function OptimizationNotice({ optimization }) {
  const { t: tt } = useTranslation();
  const { error, clearError } = optimization;
  if (!error) return null;
  const text = error.kind === "empty"
    ? tt("browser:optimize.empty")
    : error.kind === "apply_failed"
      ? tt("browser:optimize.applyFailed", { message: error.message || "" })
      : tt("browser:optimize.failed", { message: error.message || "" });
  const warn = error.kind !== "empty";
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 9,
      margin: "12px 26px 0", padding: "8px 13px", borderRadius: 10,
      fontSize: 12.5,
      background: warn ? "var(--err-bg, var(--fill))" : "var(--fill)",
      border: `1px solid ${warn ? "var(--err-line, var(--line2))" : "var(--line3)"}`,
      color: warn ? "var(--err-deep)" : "var(--tx3)" }}>
      <span style={{ flex: 1 }}>{text}</span>
      <button className="ficon-btn" title={tt("browser:round.cancel")}
        onClick={clearError}><CloseIcon /></button>
    </div>
  );
}

/** Cursor 式内联 diff:原文删除线 + 改写候选 + 理由 + 接受/拒绝。 */
export function InlineRewriteDiff({ original, candidate, onAccept, onReject }) {
  const { t: tt } = useTranslation();
  return (
    <div style={{ margin: "6px 0", borderRadius: 12, overflow: "hidden",
      border: "1px solid var(--acc-line2)" }}>
      <div className="selectable" style={{ padding: "9px 14px 9px 30px",
        fontSize: 13, lineHeight: 1.65, position: "relative",
        background: "var(--err-bg, #FDF3F3)", color: "var(--tx3)",
        textDecoration: "line-through",
        textDecorationColor: "color-mix(in srgb, var(--err) 50%, transparent)",
        borderBottom: "1px solid var(--line5)",
        whiteSpace: "pre-wrap", overflowWrap: "break-word" }}>
        <span className="mono" style={{ position: "absolute", left: 12,
          top: 9, fontWeight: 700, color: "var(--err)",
          textDecoration: "none" }}>−</span>
        {String(original || "").slice(0, 4000)}
      </div>
      <div className="selectable" style={{ padding: "9px 14px 9px 30px",
        fontSize: 13, lineHeight: 1.65, position: "relative",
        background: "var(--ok-bg)", color: "var(--ok-body2, var(--tx1))",
        whiteSpace: "pre-wrap", overflowWrap: "break-word" }}>
        <span className="mono" style={{ position: "absolute", left: 12,
          top: 9, fontWeight: 700, color: "var(--ok-deep)" }}>+</span>
        {candidate.text.slice(0, 4000)}
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 8,
        padding: "6px 10px", background: "var(--surface, var(--bg))",
        borderTop: "1px solid var(--line5)" }}>
        {candidate.reason && (
          <span style={{ fontSize: 11.5, color: "var(--tx4)", minWidth: 0,
            overflow: "hidden", textOverflow: "ellipsis",
            whiteSpace: "nowrap" }}>{candidate.reason}</span>
        )}
        <div style={{ marginLeft: "auto", display: "flex", gap: 6,
          flex: "none" }}>
          <button className="fbtn" onClick={onReject}
            style={{ height: 26, padding: "0 12px", fontSize: 12 }}>
            {tt("browser:optimize.reject")}
          </button>
          <button className="fbtn-primary" onClick={onAccept}
            style={{ height: 26, padding: "0 12px", fontSize: 12,
              background: "var(--ok)" }}>
            {tt("browser:optimize.accept")}
          </button>
        </div>
      </div>
    </div>
  );
}

/** 底部悬浮汇总栏:diff 待处理时是接受/拒绝栏,多选时是选择栏。 */
export function OptimizationFloatBar({
  pendingCount,
  onPrev,
  onNext,
  onAcceptAll,
  onRejectAll,
  applying,
  selection,
}) {
  const { t: tt } = useTranslation();
  const barStyle = {
    position: "absolute", left: "50%", bottom: 24,
    transform: "translateX(-50%)", zIndex: 20,
    display: "flex", alignItems: "center", gap: 10,
    background: "var(--accent)", color: "var(--accent-fg)",
    borderRadius: 12, padding: "8px 10px 8px 15px",
    boxShadow: "0 12px 36px rgba(0,0,0,.3)", fontSize: 12.5,
    whiteSpace: "nowrap",
  };
  const ghostBtn = {
    border: "none", borderRadius: 8, padding: "5px 12px", fontSize: 12,
    fontWeight: 600, cursor: "default",
    background: "color-mix(in srgb, var(--accent-fg) 13%, transparent)",
    color: "var(--accent-fg)",
  };
  if (applying) {
    return (
      <div style={barStyle}>
        <Spinner size={13} />
        <span>{tt("browser:optimize.applying")}</span>
      </div>
    );
  }
  if (pendingCount > 0) {
    return (
      <div style={barStyle}>
        <span style={{ fontWeight: 650 }}>
          {tt("browser:optimize.pending", { n: pendingCount })}
        </span>
        <span style={{ display: "flex", gap: 2 }}>
          <button style={{ ...ghostBtn, padding: "5px 8px" }}
            title={tt("browser:optimize.prev")} onClick={onPrev}>↑</button>
          <button style={{ ...ghostBtn, padding: "5px 8px" }}
            title={tt("browser:optimize.next")} onClick={onNext}>↓</button>
        </span>
        <span style={{ width: 1, height: 18,
          background: "color-mix(in srgb, var(--accent-fg) 18%, transparent)" }} />
        <button style={ghostBtn} onClick={onRejectAll}>
          {tt("browser:optimize.rejectAll")}
          <span className="mono" style={{ fontSize: 10, opacity: .7,
            marginLeft: 5 }}>⌘⌫</span>
        </button>
        <button style={{ ...ghostBtn, background: "var(--ok)", color: "#fff" }}
          onClick={onAcceptAll}>
          {tt("browser:optimize.acceptAll")}
          <span className="mono" style={{ fontSize: 10, opacity: .75,
            marginLeft: 5 }}>⌘⏎</span>
        </button>
      </div>
    );
  }
  if (selection && selection.count > 0) {
    return (
      <div style={barStyle}>
        <span style={{ fontWeight: 650 }}>
          {tt("browser:optimize.selected", { n: selection.count })}
        </span>
        <span style={{ fontSize: 11,
          color: "color-mix(in srgb, var(--accent-fg) 60%, transparent)" }}>
          {tt("browser:optimize.selectHint")}
        </span>
        <span style={{ width: 1, height: 18,
          background: "color-mix(in srgb, var(--accent-fg) 18%, transparent)" }} />
        <button style={ghostBtn} onClick={selection.onCancel}>
          {tt("browser:round.cancel")}
        </button>
        <button style={{ ...ghostBtn,
          background: roleColorVar(selection.roleColor), color: "#fff" }}
          onClick={selection.onRun}>
          {tt("browser:optimize.optimizeSelected", { role: selection.roleName })}
        </button>
      </div>
    );
  }
  return null;
}

/** 右侧缩略指示条:待处理 diff 在整个时间线里的位置,点击跳转。 */
export function OptimizationMinimap({ scrollRef, pendingTurns, onJump }) {
  const { t: tt } = useTranslation();
  const [view, setView] = useState(null); // {top, height} 百分比
  const [marks, setMarks] = useState([]); // [{turn, top}] 百分比

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !pendingTurns.length) { setMarks([]); setView(null); return; }
    const measure = () => {
      const total = el.scrollHeight || 1;
      setView({
        top: (el.scrollTop / total) * 100,
        height: Math.max((el.clientHeight / total) * 100, 4),
      });
      setMarks(pendingTurns.flatMap(turn => {
        const node = el.querySelector(`[data-round="${turn}"]`);
        if (!node) return [];
        return [{ turn, top: ((node.offsetTop + node.offsetHeight / 2) / total) * 100 }];
      }));
    };
    measure();
    el.addEventListener("scroll", measure, { passive: true });
    window.addEventListener("resize", measure);
    return () => {
      el.removeEventListener("scroll", measure);
      window.removeEventListener("resize", measure);
    };
  }, [scrollRef, pendingTurns]);

  if (!marks.length) return null;
  return (
    <div style={{ position: "absolute", right: 6, top: "12%", height: "72%",
      width: 10, zIndex: 15 }}>
      <div style={{ position: "absolute", top: 0, bottom: 0, left: 3.5,
        width: 3, background: "var(--line2)", opacity: .55, borderRadius: 2 }} />
      {view && (
        <div style={{ position: "absolute", left: 0, width: 10,
          borderRadius: 5, background: "color-mix(in srgb, var(--tx1) 10%, transparent)",
          top: `${view.top}%`, height: `${view.height}%`,
          pointerEvents: "none" }} />
      )}
      {marks.map(mark => (
        <button
          key={mark.turn}
          title={tt("browser:optimize.jumpTo", { n: mark.turn })}
          onClick={() => onJump(mark.turn)}
          style={{ position: "absolute", left: -2, width: 14, height: 8,
            borderRadius: 4, border: "2px solid var(--surface, var(--bg))",
            background: "var(--role-violet)", cursor: "default", padding: 0,
            top: `calc(${Math.min(Math.max(mark.top, 0), 99)}% - 4px)`,
            boxShadow: "0 1px 4px rgba(0,0,0,.25)" }}
        />
      ))}
    </div>
  );
}
