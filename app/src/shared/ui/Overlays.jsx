// 其余弹层:差异预览 / 原地修改确认 / 结果 toast /
// 三个筛选弹层 / 快速上手引导(设置与数据来源已合并进 Settings.jsx 全屏页)
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CloseIcon, SearchIcon, Spinner, ToolIcon } from "./icons.jsx";
import { ConfirmBox } from "./ConfirmBox.jsx";

// ---------- 会话搜索命令面板(⌘K 风格居中浮层) ----------
export function SearchPalette({ placeholder, query, onQuery, results,
  recentLabel, emptyLabel, onClose }) {
  const [sel, setSel] = useState(0);
  useEffect(() => setSel(0), [query]);
  useEffect(() => {
    const onKey = e => {
      if (e.key === "Escape") { e.preventDefault(); onClose(); }
      else if (e.key === "ArrowDown") { e.preventDefault(); setSel(s => Math.min(s + 1, results.length - 1)); }
      else if (e.key === "ArrowUp") { e.preventDefault(); setSel(s => Math.max(s - 1, 0)); }
      else if (e.key === "Enter") { e.preventDefault(); results[sel]?.onClick?.(); onClose(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [results, sel, onClose]);
  return (
    <div onClick={onClose}
      style={{ position: "absolute", inset: 0, zIndex: 70, background: "var(--dim)",
        display: "flex", justifyContent: "center", alignItems: "flex-start", paddingTop: "9vh" }}>
      <div onClick={e => e.stopPropagation()} className="fsheet"
        style={{ width: "min(680px, 78vw)", maxHeight: "76vh", display: "flex", flexDirection: "column",
          background: "var(--bg)", borderRadius: 14, boxShadow: "var(--shadow-sheet)", overflow: "hidden" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 11, padding: "0 14px",
          height: 52, borderBottom: "1px solid var(--line5)", flex: "none" }}>
          <span style={{ color: "var(--tx4)", display: "inline-flex" }}><SearchIcon /></span>
          <input autoFocus value={query} onChange={onQuery} placeholder={placeholder}
            style={{ flex: 1, border: "none", background: "transparent", fontSize: 15,
              color: "var(--tx1)", outline: "none" }} />
          <button className="ftool-btn" onClick={onClose}><CloseIcon size={13} /></button>
        </div>
        <div className="fscroll" style={{ overflowY: "auto", padding: "8px", minHeight: 0 }}>
          {recentLabel && (
            <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx4)",
              padding: "6px 10px 4px" }}>{recentLabel}</div>
          )}
          {results.length === 0 ? (
            <div style={{ padding: "26px 12px", textAlign: "center", color: "var(--tx5)",
              fontSize: 13 }}>{emptyLabel}</div>
          ) : results.map((r, i) => (
            <div key={r.id} onMouseEnter={() => setSel(i)}
              onClick={() => { r.onClick?.(); onClose(); }}
              style={{ display: "flex", alignItems: "center", gap: 10, padding: "0 10px", height: 42,
                borderRadius: 8, cursor: "default",
                background: i === sel ? "var(--acc-soft2)" : "transparent" }}>
              {r.tool && <ToolIcon tool={r.tool} size={20} />}
              <span style={{ flex: 1, minWidth: 0, fontSize: 13, color: "var(--tx1)",
                whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{r.title}</span>
              {r.meta && <span className="mono" style={{ fontSize: 11, color: "var(--tx5)", flex: "none",
                maxWidth: "42%", whiteSpace: "nowrap", overflow: "hidden",
                textOverflow: "ellipsis" }}>{r.meta}</span>}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ---------- 会话右键菜单 ----------
export function ContextMenu({ x, y, items, onClose }) {
  const width = 208;
  const height = items.reduce((a, it) => a + (it.sep ? 9 : 30), 12);
  const left = Math.max(8, Math.min(x, window.innerWidth - width - 8));
  const top = Math.max(8, Math.min(y, window.innerHeight - height - 8));
  return (
    <>
      <div onClick={onClose} onContextMenu={e => { e.preventDefault(); onClose(); }}
        style={{ position: "absolute", inset: 0, zIndex: 55 }} />
      <div style={{ position: "absolute", left, top, width, zIndex: 56, padding: 6,
        background: "var(--bg)", borderRadius: 10,
        boxShadow: "var(--shadow-menu)",
         }}>
        {items.map((it, i) => it.sep
          ? <div key={i} style={{ height: 1, background: "var(--line3)", margin: "4px 8px" }} />
          : (
            <div key={i} className={it.disabled ? undefined : "hov-item"}
              onClick={() => { if (it.disabled) return; onClose(); it.onClick?.(); }}
              title={it.disabled ? it.disabledHint : undefined}
              style={{ display: "flex", alignItems: "center", gap: 8, height: 30,
                padding: "0 9px", borderRadius: 6, fontSize: 12,
                color: it.disabled ? "var(--tx5)" : it.danger ? "var(--err-text)" : "var(--tx2)",
                cursor: it.disabled ? "default" : "pointer", whiteSpace: "nowrap" }}>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>{it.label}</span>
              {it.hint && <span style={{ fontSize: 11, color: "var(--tx5)", flex: "none" }}>{it.hint}</span>}
            </div>
          ))}
      </div>
    </>
  );
}

// ---------- 输入弹框(重命名 / 标签) ----------
export function PromptBox({ title, desc, placeholder, initial, confirmLabel,
  onCancel, onConfirm }) {
  const { t } = useTranslation();
  const [val, setVal] = useState(initial || "");
  const submit = () => onConfirm(val.trim());
  return (
    <ConfirmBox width={420} title={title} actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>{t("overlays:prompt.cancel")}</button>
      <button className="fbtn-primary" style={{ height: 34, padding: "0 16px", fontSize: 13 }}
        onClick={submit}>{confirmLabel || t("overlays:prompt.confirm")}</button>
    </>}>
      {desc && <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 7,
        lineHeight: 1.5 }}>{desc}</div>}
      <input autoFocus value={val} placeholder={placeholder}
        onChange={e => setVal(e.target.value)}
        onKeyDown={e => { if (e.key === "Enter") submit(); }}
        style={{ width: "100%", boxSizing: "border-box", height: 34, marginTop: 12,
          padding: "0 11px", background: "var(--surface)", border: "1px solid var(--line)",
          borderRadius: 8, fontSize: 13, color: "var(--tx1)" }} />
    </ConfirmBox>
  );
}

// ---------- 结果 toast ----------
export function Toast({ toast, onDismiss }) {
  const kind = toast.kind;
  const bg = kind === "fail" ? "var(--err-bg)" : kind === "ok" ? "var(--ok-bg)" : "var(--bg)";
  const border = kind === "fail" ? "var(--err-line)" : kind === "ok" ? "var(--ok-line)" : "var(--line3)";
  const color = kind === "fail" ? "var(--err-text)" : kind === "ok" ? "var(--ok-deep)" : "var(--tx2)";
  return (
    <div style={{ position: "absolute", left: "50%", bottom: 26, transform: "translateX(-50%)",
      zIndex: 45, display: "flex", alignItems: "center", gap: 11, padding: "12px 16px",
      borderRadius: 10, background: bg, border: `1px solid ${border}`,
      boxShadow: "var(--shadow-sheet)", maxWidth: 560 }}>
      {kind === "run" ? <Spinner size={20} track="var(--line)" /> : (
        <span style={{ width: 26, height: 26, flex: "none", borderRadius: "50%",
          background: kind === "ok" ? "var(--ok)" : "var(--err)", display: "inline-flex",
          alignItems: "center", justifyContent: "center", color: "#fff", fontSize: 14 }}>
          {kind === "ok" ? "✓" : "×"}</span>)}
      <div>
        <div style={{ fontSize: 13, fontWeight: 600, color }}>{toast.title}</div>
        <div style={{ fontSize: 11, color: "var(--tx3b)", marginTop: 2 }}>{toast.desc}</div>
      </div>
      {toast.action && (
        <button className="fbtn" style={{ height: 28, padding: "0 12px", fontSize: 12,
          flex: "none", fontWeight: 600 }}
          onClick={toast.action.onClick}>{toast.action.label}</button>)}
      <a onClick={onDismiss} style={{ color: "var(--tx5)", fontSize: 16, marginLeft: 6 }}>×</a>
    </div>
  );
}
