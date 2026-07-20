// 其余弹层:差异预览 / 原地修改确认 / 快照还原确认 / 结果 toast /
// 三个筛选弹层 / 快速上手引导(设置与数据来源已合并进 Settings.jsx 全屏页)
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { renderEvents } from "../../api/contract/events.js";
import { TOOL_NAME, TOOLS } from "../../api/contract/tools.js";
import { STATUS_CODE } from "../../features/migration/migrationModel.js";
import { ACCENT, fmtSize } from "../../domain/tools/toolDisplay.js";
import { fmtTime } from "../../domain/sessions/sessionModel.js";
import { Spinner, ToolIcon } from "./icons.jsx";
import { CheckSquare, RadioDot, Sheet } from "./primitives.jsx";

// ---------- 差异预览 ----------
export function DiffSheet({ ops, preview, loading, error, onClose }) {
  const { t } = useTranslation();
  const replyText = items => {
    const limit = 8000;
    let text = "";
    for (const item of items || []) {
      const input = typeof item.input === "string" ? item.input : JSON.stringify(item.input, null, 2);
      const part = item.kind === "text"
        ? `${t("overlays:diff.replyTextLabel")}\n${item.text}`
        : `${t("overlays:diff.replyToolLabel", { name: item.name })}\n${t("overlays:diff.replyParamsLabel")} ${input}\n${t("overlays:diff.replyOutputLabel")}\n${item.output}`;
      const room = limit - text.length;
      if (room <= 0) break;
      text += (text ? "\n\n" : "") + part.slice(0, room);
    }
    return text.length >= limit ? `${text.slice(0, limit)}\n${t("overlays:diff.previewTruncated")}` : text;
  };
  return (
    <Sheet width={760} maxHeight={780} onClose={onClose}>
      <div style={{ flex: "none", padding: "15px 20px", borderBottom: "1px solid var(--line5)",
        display: "flex", alignItems: "center" }}>
        <div style={{ fontSize: 14.5, fontWeight: 650 }}>{t("overlays:diff.title")}</div>
        <div style={{ fontSize: 12, color: "var(--tx4)", marginLeft: 12 }}>
          {t("overlays:diff.metaOps", { n: ops.length })}
          {preview && `${t("overlays:diff.metaSize", { before: fmtSize(preview.before.size), after: fmtSize(preview.after.size) })}
            ${t("overlays:diff.metaCount", { before: preview.before.count, after: preview.after.count })}`}
        </div>
        <div style={{ flex: 1 }} />
        <a onClick={onClose} style={{ color: "var(--tx5)", fontSize: 18 }}>×</a>
      </div>
      <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "18px 20px" }}>
        {ops.length === 0 && (
          <div style={{ textAlign: "center", color: "var(--tx5)", fontSize: 13, padding: 40 }}>{t("overlays:diff.empty")}</div>)}
        {loading && (
          <div style={{ display: "flex", alignItems: "center", gap: 10, color: "var(--tx4)",
            fontSize: 12.5, marginBottom: 14 }}><Spinner size={14} /> {t("overlays:diff.loading")}</div>)}
        {error && (
          <div style={{ padding: "9px 12px", borderRadius: 8, background: "var(--err-bg2)",
            color: "var(--err-text)", fontSize: 12, marginBottom: 12 }}>{error}</div>)}
        {ops.map(o => (
          <div key={o.id} style={{ border: "1px solid var(--line3)", borderRadius: 10, overflow: "hidden",
            marginBottom: 12 }}>
            <div style={{ padding: "9px 13px", background: "var(--fill2)", borderBottom: "1px solid var(--line5)",
              display: "flex", alignItems: "center", gap: 9 }}>
              <span style={{ width: 7, height: 7, borderRadius: "50%", background: o.dot }} />
              <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx2)" }}>{o.label}</span>
              {o.type === "rewrite" && o.text === o.orig && (
                <span style={{ fontSize: 11, color: "var(--warn-deep)", marginLeft: "auto" }}>{t("overlays:diff.contentUnchanged")}</span>)}
            </div>
            <div className="mono selectable" style={{ padding: "11px 13px", fontSize: 11.5, lineHeight: 1.7 }}>
              <div className="fscroll" style={{ background: "var(--err-bg2)", color: "var(--err-text)",
                 padding: "6px 10px", borderRadius: 5, whiteSpace: "pre-wrap", overflowWrap: "break-word",
                 maxHeight: 180, overflowY: "auto" }}>− {o.type === "assistant-reply"
                  ? replyText(o.origItems).slice(0, 8000)
                  : (o.orig || t("overlays:diff.noUserMessage")).slice(0, 4000)}</div>
              {o.type === "delete" ? (
                <div style={{ fontSize: 11, color: "var(--tx5)", marginTop: 6 }}>{o.summary}</div>
              ) : (
                <div className="fscroll" style={{ background: "var(--ok-bg2)", color: "var(--ok-body2)",
                  padding: "6px 10px", borderRadius: 5, marginTop: 5, whiteSpace: "pre-wrap",
                  overflowWrap: "break-word", maxHeight: 180, overflowY: "auto" }}>
                  + {o.type === "assistant-reply"
                    ? replyText(o.items.map(item => item.kind === "tool"
                      ? { ...item, input: item.inputText } : item)).slice(0, 8000)
                    : (o.text || "").slice(0, 4000)}</div>
              )}
            </div>
          </div>
        ))}
        {preview?.changes?.length > 0 && (
          <div style={{ fontSize: 12, color: "var(--tx3b)", lineHeight: 1.6 }}>
            {t("overlays:diff.engineConfirm", { changes: renderEvents(preview.changes).join(";") })}</div>)}
      </div>
      <div style={{ flex: "none", padding: "13px 20px", borderTop: "1px solid var(--line5)",
        display: "flex", justifyContent: "flex-end" }}>
        <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onClose}>{t("overlays:diff.close")}</button>
      </div>
    </Sheet>
  );
}

// ---------- 小确认框 ----------
function ConfirmBox({ width = 400, title, children, actions }) {
  return (
    <div style={{ position: "absolute", inset: 0, background: "var(--scrim)", display: "flex",
      alignItems: "center", justifyContent: "center", zIndex: 44, animation: "ffade .15s ease" }}>
      <div style={{ width, background: "var(--bg)", borderRadius: 12,
        boxShadow: "0 24px 60px -18px rgba(20,28,38,.5)", padding: 22, animation: "fsheet .2s ease" }}>
        <div style={{ fontSize: 15, fontWeight: 650 }}>{title}</div>
        {children}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 10, marginTop: 18 }}>{actions}</div>
      </div>
    </div>
  );
}

export function ApplyConfirm({ ops, saveMode, setSaveMode, editCaps, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const modes = [
    ["saveas", t("overlays:apply.saveas"), t("overlays:apply.saveasDesc")],
    ["inplace", t("overlays:apply.inplace"), t("overlays:apply.inplaceDesc")],
  ].filter(([mode]) => {
    return ops.every(op => op.modes?.includes(mode) ||
      editCaps?.operation_modes?.[op.backendOp || (op.type === "delete" ? "delete-turn" : "rewrite")]?.includes(mode));
  });
  const inplace = saveMode === "inplace";
  return (
    <ConfirmBox width={440} title={t("overlays:apply.title", { n: ops.length })} actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>{t("overlays:apply.cancel")}</button>
      {inplace ? (
        <button style={{ height: 34, padding: "0 16px", background: "var(--err2)", border: "none",
          borderRadius: 8, fontSize: 13, color: "#fff", cursor: "pointer", fontWeight: 600 }}
          onClick={onConfirm}>{t("overlays:apply.confirmInplace")}</button>
      ) : (
        <button className="fbtn-primary" style={{ height: 34, padding: "0 16px", fontSize: 13 }}
          onClick={onConfirm}>{t("overlays:apply.confirmSaveas")}</button>
      )}
    </>}>
      <div style={{ marginTop: 12 }}>
        {modes.map(([k, l, d]) => {
          const on = saveMode === k;
          return (
            <label key={k} onClick={() => setSaveMode(k)}
              style={{ display: "flex", alignItems: "flex-start", gap: 9, padding: "9px 11px",
                border: `1px solid ${on ? ACCENT : "var(--line3)"}`,
                background: on ? "var(--acc-soft4)" : "var(--surface)",
                borderRadius: 9, marginTop: 7, cursor: "pointer" }}>
              <span style={{ marginTop: 1, display: "inline-flex" }}><RadioDot on={on} /></span>
              <span>
                <span style={{ fontSize: 12.5, color: "var(--tx2)", fontWeight: 500 }}>{l}</span><br />
                <span style={{ fontSize: 11, color: "var(--tx5)" }}>{d}</span>
              </span>
            </label>
          );
        })}
      </div>
      <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 12, lineHeight: 1.55 }}>
        {inplace ? t("overlays:apply.inplaceFootnote") : t("overlays:apply.saveasFootnote")}</div>
    </ConfirmBox>
  );
}

export function SnapRestoreConfirm({ snap, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const bullets = [
    ["var(--warn)", t("overlays:snapRestore.bullet1")],
    ["var(--ok)", t("overlays:snapRestore.bullet2")],
    ["var(--accent)", t("overlays:snapRestore.bullet3")],
    ["var(--info-dot)", t("overlays:snapRestore.bullet4")],
  ];
  return (
    <ConfirmBox width={440} title={t("overlays:snapRestore.title")} actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>{t("overlays:snapRestore.cancel")}</button>
      <button className="fbtn-primary" style={{ height: 34, padding: "0 16px", fontSize: 13 }}
        onClick={onConfirm}>{t("overlays:snapRestore.confirm")}</button>
    </>}>
      <div style={{ fontSize: 12.5, color: "var(--tx3b)", marginTop: 7, lineHeight: 1.5 }}>
        {t("overlays:snapRestore.desc", { title: snap.title, time: fmtTime(snap.time) })}</div>
      <div style={{ marginTop: 14, border: "1px solid var(--line3)", borderRadius: 10, padding: "12px 14px",
        display: "flex", flexDirection: "column", gap: 9 }}>
        {bullets.map(([c, txt], i) => (
          <div key={i} style={{ display: "flex", gap: 9, fontSize: 12, color: "var(--tx2b)", lineHeight: 1.45 }}>
            <span style={{ width: 5, height: 5, borderRadius: "50%", background: c, flex: "none",
              marginTop: 6 }} />{txt}
          </div>
        ))}
      </div>
    </ConfirmBox>
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
        boxShadow: "0 16px 40px -14px rgba(20,28,38,.42),0 0 0 1px var(--ring)",
        animation: "fpop .12s ease" }}>
        {items.map((it, i) => it.sep
          ? <div key={i} style={{ height: 1, background: "var(--line3)", margin: "4px 8px" }} />
          : (
            <div key={i} className={it.disabled ? undefined : "hov-item"}
              onClick={() => { if (it.disabled) return; onClose(); it.onClick?.(); }}
              title={it.disabled ? it.disabledHint : undefined}
              style={{ display: "flex", alignItems: "center", gap: 8, height: 30,
                padding: "0 9px", borderRadius: 7, fontSize: 12.5,
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

// ---------- 删除会话确认 ----------
export function SessionDeleteConfirm({ sess, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const subCount = (sess.tree_count || 1) - 1;
  const oc = sess.tool === "opencode";
  const bullets = [
    subCount > 0 && ["var(--warn)", t("overlays:delete.bulletSub", { n: subCount })],
    ["var(--ok)", t("overlays:delete.bulletSnapshot")],
    oc
      ? ["var(--err)", t("overlays:delete.bulletOpenCode")]
      : ["var(--accent)", t("overlays:delete.bulletUndoable")],
  ].filter(Boolean);
  return (
    <ConfirmBox width={430} title={t("overlays:delete.title")} actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>{t("overlays:delete.cancel")}</button>
      <button style={{ height: 34, padding: "0 16px", background: "var(--err2)", border: "none",
        borderRadius: 8, fontSize: 13, color: "#fff", cursor: "pointer", fontWeight: 600 }}
        onClick={onConfirm}>{oc ? t("overlays:delete.confirmOpenCode") : t("overlays:delete.confirmOther")}</button>
    </>}>
      <div style={{ fontSize: 12.5, color: "var(--tx3b)", marginTop: 7, lineHeight: 1.5 }}>
        {t("overlays:delete.desc", { title: sess.title || sess.id, tool: TOOL_NAME[sess.tool] })}</div>
      <div style={{ marginTop: 14, border: "1px solid var(--line3)", borderRadius: 10, padding: "12px 14px",
        display: "flex", flexDirection: "column", gap: 9 }}>
        {bullets.map(([c, txt], i) => (
          <div key={i} style={{ display: "flex", gap: 9, fontSize: 12, color: "var(--tx2b)", lineHeight: 1.45 }}>
            <span style={{ width: 5, height: 5, borderRadius: "50%", background: c, flex: "none",
              marginTop: 6 }} />{txt}
          </div>
        ))}
      </div>
    </ConfirmBox>
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
      {desc && <div style={{ fontSize: 12.5, color: "var(--tx3b)", marginTop: 7,
        lineHeight: 1.5 }}>{desc}</div>}
      <input autoFocus value={val} placeholder={placeholder}
        onChange={e => setVal(e.target.value)}
        onKeyDown={e => { if (e.key === "Enter") submit(); }}
        style={{ width: "100%", boxSizing: "border-box", height: 34, marginTop: 12,
          padding: "0 11px", background: "var(--surface)", border: "1px solid var(--line)",
          borderRadius: 8, fontSize: 13, color: "var(--tx1)", outline: "none" }} />
    </ConfirmBox>
  );
}

// ---------- 批量删除确认 ----------
export function BatchDeleteConfirm({ sessions, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const ocCount = sessions.filter(s => s.tool === "opencode").length;
  const bullets = [
    ["var(--ok)", t("overlays:delete.bulletBatchSnapshot")],
    ocCount > 0 && ["var(--err)", t("overlays:delete.bulletBatchOpenCode", { n: ocCount })],
    ["var(--accent)", t("overlays:delete.bulletBatchRest")],
  ].filter(Boolean);
  return (
    <ConfirmBox width={430} title={t("overlays:delete.batchTitle", { n: sessions.length })} actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>{t("overlays:delete.cancel")}</button>
      <button style={{ height: 34, padding: "0 16px", background: "var(--err2)", border: "none",
        borderRadius: 8, fontSize: 13, color: "#fff", cursor: "pointer", fontWeight: 600 }}
        onClick={onConfirm}>{t("overlays:delete.confirmOther")}</button>
    </>}>
      <div style={{ marginTop: 14, border: "1px solid var(--line3)", borderRadius: 10,
        padding: "12px 14px", display: "flex", flexDirection: "column", gap: 9 }}>
        {bullets.map(([c, txt], i) => (
          <div key={i} style={{ display: "flex", gap: 9, fontSize: 12, color: "var(--tx2b)",
            lineHeight: 1.45 }}>
            <span style={{ width: 5, height: 5, borderRadius: "50%", background: c, flex: "none",
              marginTop: 6 }} />{txt}
          </div>
        ))}
      </div>
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
      boxShadow: "0 12px 30px -12px rgba(20,28,38,.4)", animation: "fsheet .22s ease", maxWidth: 560 }}>
      {kind === "run" ? <Spinner size={20} track="var(--line)" /> : (
        <span style={{ width: 26, height: 26, flex: "none", borderRadius: "50%",
          background: kind === "ok" ? "var(--ok)" : "var(--err)", display: "inline-flex",
          alignItems: "center", justifyContent: "center", color: "#fff", fontSize: 14 }}>
          {kind === "ok" ? "✓" : "×"}</span>)}
      <div>
        <div style={{ fontSize: 13, fontWeight: 600, color }}>{toast.title}</div>
        <div style={{ fontSize: 11.5, color: "var(--tx3b)", marginTop: 2 }}>{toast.desc}</div>
      </div>
      {toast.action && (
        <button className="fbtn" style={{ height: 28, padding: "0 12px", fontSize: 12,
          flex: "none", fontWeight: 600 }}
          onClick={toast.action.onClick}>{toast.action.label}</button>)}
      <a onClick={onDismiss} style={{ color: "var(--tx5)", fontSize: 16, marginLeft: 6 }}>×</a>
    </div>
  );
}

// ---------- 筛选弹层(共用外壳) ----------
function PopShell({ onClose, onClear, children, t }) {
  return (
    <>
      <div onClick={onClose} style={{ position: "absolute", inset: 0, zIndex: 35 }} />
      <div style={{ position: "absolute", left: 66, top: 190, width: 272, zIndex: 36,
        background: "var(--bg)", borderRadius: 11,
        boxShadow: "0 16px 40px -14px rgba(20,28,38,.42),0 0 0 1px var(--ring)",
        overflow: "hidden", animation: "fpop .14s ease" }}>
        <div className="fscroll" style={{ maxHeight: 430, overflowY: "auto", padding: "12px 13px" }}>
          {children}
        </div>
        <div style={{ display: "flex", alignItems: "center", padding: "9px 13px",
          borderTop: "1px solid var(--line5)" }}>
          <a onClick={onClear} style={{ fontSize: 11.5, color: "var(--tx3b)" }}>{t("overlays:filter.clear")}</a>
          <span style={{ flex: 1 }} />
          <button className="fbtn-primary" style={{ height: 28, padding: "0 14px", fontSize: 12 }}
            onClick={onClose}>{t("overlays:filter.done")}</button>
        </div>
      </div>
    </>
  );
}

const SectionTitle = ({ children, first }) => (
  <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx5)", letterSpacing: ".03em",
    margin: first ? "0 0 6px" : "12px 0 6px" }}>{children}</div>
);

function CheckRow({ on, onClick, icon, label, extra }) {
  return (
    <div className="hov-item" onClick={onClick}
      style={{ display: "flex", alignItems: "center", gap: 9, padding: "6px 7px", borderRadius: 7,
        cursor: "pointer" }}>
      <CheckSquare on={on} />
      {icon}
      <span style={{ fontSize: 12.5, color: "var(--tx2)", flex: 1, whiteSpace: "nowrap",
        overflow: "hidden", textOverflow: "ellipsis" }}>{label}</span>
      {extra && <span style={{ fontSize: 11, color: "var(--tx5)" }}>{extra}</span>}
    </div>
  );
}

function RadioRow({ on, onClick, label }) {
  return (
    <div className="hov-item" onClick={onClick}
      style={{ display: "flex", alignItems: "center", gap: 9, padding: "6px 7px", borderRadius: 7,
        cursor: "pointer" }}>
      <RadioDot on={on} />
      <span style={{ fontSize: 12.5, color: "var(--tx2)", whiteSpace: "nowrap", overflow: "hidden",
        textOverflow: "ellipsis" }}>{label}</span>
    </div>
  );
}

// 会话库筛选:来源 / 时间 / 目录
export function LibraryFilter({ f, setF, counts, dirs, tags = [], onClose, onClear }) {
  const { t } = useTranslation();
  const times = [["all", t("overlays:filter.allTime")], ["today", t("overlays:filter.today")],
    ["last7", t("overlays:filter.last7")], ["last30", t("overlays:filter.last30")]];
  return (
    <PopShell onClose={onClose} onClear={onClear} t={t}>
      <SectionTitle first>{t("overlays:filter.source")}</SectionTitle>
      {TOOLS.map(t2 => (
        <CheckRow key={t2} on={f.src.includes(t2)} icon={<ToolIcon tool={t2} size={24} />}
          label={TOOL_NAME[t2]} extra={counts[t2] || 0}
          onClick={() => setF(v => ({ ...v, src: v.src.includes(t2)
            ? v.src.filter(x => x !== t2) : [...v.src, t2] }))} />
      ))}
      <SectionTitle>{t("overlays:filter.timeRange")}</SectionTitle>
      {times.map(([k, l]) => (
        <RadioRow key={k} on={f.time === k} label={l}
          onClick={() => setF(v => ({ ...v, time: k }))} />
      ))}
      <SectionTitle>{t("overlays:filter.projectDir")}</SectionTitle>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
        {dirs.map(d => {
          const on = f.dir === d;
          return (
            <button key={d} className="mono" onClick={() => setF(v => ({ ...v, dir: on ? null : d }))}
              style={{ height: 24, padding: "0 9px", borderRadius: 20,
                border: `1px solid ${on ? ACCENT : "var(--line)"}`, background: on ? "var(--acc-soft)" : "var(--surface)",
                color: on ? ACCENT : "var(--tx3)", fontSize: 11, cursor: "pointer" }}>{d}</button>
          );
        })}
        {dirs.length === 0 && <span style={{ fontSize: 11.5, color: "var(--tx5)" }}>{t("overlays:filter.noDirs")}</span>}
      </div>
      {tags.length > 0 && (<>
        <SectionTitle>{t("overlays:filter.tags")}</SectionTitle>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
          {tags.map(t2 => {
            const on = f.tag === t2;
            return (
              <button key={t2} onClick={() => setF(v => ({ ...v, tag: on ? null : t2 }))}
                style={{ height: 24, padding: "0 9px", borderRadius: 20,
                  border: `1px solid ${on ? ACCENT : "var(--line)"}`,
                  background: on ? "var(--acc-soft)" : "var(--surface)",
                  color: on ? ACCENT : "var(--tx3)", fontSize: 11, cursor: "pointer" }}>{t2}</button>
            );
          })}
        </div>
      </>)}
      <SectionTitle>{t("overlays:filter.content")}</SectionTitle>
      <CheckRow on={f.mig} label={t("overlays:filter.onlyMigrated")}
        onClick={() => setF(v => ({ ...v, mig: !v.mig }))} />
      <CheckRow on={f.sub} label={t("overlays:filter.onlySubSessions")}
        onClick={() => setF(v => ({ ...v, sub: !v.sub }))} />
      <CheckRow on={f.arch} label={t("overlays:filter.showArchived")}
        onClick={() => setF(v => ({ ...v, arch: !v.arch }))} />
    </PopShell>
  );
}

// 迁移历史筛选:来源 / 目标 / 状态 / 时间
export function HistoryFilter({ f, setF, onClose, onClear }) {
  const { t } = useTranslation();
  const statusOptions = [
    [STATUS_CODE.success, t(`common:${STATUS_CODE.success}`)],
    [STATUS_CODE.failed, t(`common:${STATUS_CODE.failed}`)],
    [STATUS_CODE.rolledBack, t(`common:${STATUS_CODE.rolledBack}`)],
  ];
  return (
    <PopShell onClose={onClose} onClear={onClear} t={t}>
      <SectionTitle first>{t("overlays:filter.sourceTools")}</SectionTitle>
      {TOOLS.map(t2 => (
        <CheckRow key={t2} on={f.src.includes(t2)} icon={<ToolIcon tool={t2} size={24} />}
          label={TOOL_NAME[t2]}
          onClick={() => setF(v => ({ ...v, src: v.src.includes(t2)
            ? v.src.filter(x => x !== t2) : [...v.src, t2] }))} />
      ))}
      <SectionTitle>{t("overlays:filter.targetTool")}</SectionTitle>
      {[["all", t("overlays:filter.allTargets")], ...TOOLS.map(t2 => [t2, TOOL_NAME[t2]])].map(([k, l]) => (
        <RadioRow key={k} on={f.target === k} label={l}
          onClick={() => setF(v => ({ ...v, target: k }))} />
      ))}
      <SectionTitle>{t("overlays:filter.status")}</SectionTitle>
      <RadioRow key="all" on={f.status === "all"} label={t("common:status.all")}
        onClick={() => setF(v => ({ ...v, status: "all" }))} />
      {statusOptions.map(([k, l]) => (
        <RadioRow key={k} on={f.status === k} label={l}
          onClick={() => setF(v => ({ ...v, status: k }))} />
      ))}
      <SectionTitle>{t("overlays:filter.timeRange")}</SectionTitle>
      {[["all", t("overlays:filter.allTime")], ["today", t("overlays:filter.today")],
        ["yesterday", t("overlays:filter.yesterday")], ["earlier", t("overlays:filter.earlier")]].map(([k, l]) => (
        <RadioRow key={k} on={f.time === k} label={l}
          onClick={() => setF(v => ({ ...v, time: k }))} />
      ))}
    </PopShell>
  );
}

// 快照筛选:来源工具 / 创建原因 / 关联会话 / 时间
export function SnapFilter({ f, setF, sessions, reasons, onClose, onClear }) {
  const { t } = useTranslation();
  return (
    <PopShell onClose={onClose} onClear={onClear} t={t}>
      <SectionTitle first>{t("overlays:filter.sourceTools")}</SectionTitle>
      {TOOLS.map(t2 => (
        <CheckRow key={t2} on={f.src.includes(t2)} icon={<ToolIcon tool={t2} size={24} />}
          label={TOOL_NAME[t2]}
          onClick={() => setF(v => ({ ...v, src: v.src.includes(t2)
            ? v.src.filter(x => x !== t2) : [...v.src, t2] }))} />
      ))}
      <SectionTitle>{t("overlays:filter.createReason")}</SectionTitle>
      {[["all", t("overlays:filter.allReasons")], ...reasons.map(r => [r, r])].map(([k, l]) => (
        <RadioRow key={k} on={f.reason === k} label={l}
          onClick={() => setF(v => ({ ...v, reason: k }))} />
      ))}
      <SectionTitle>{t("overlays:filter.relatedSession")}</SectionTitle>
      {[["all", t("overlays:filter.allSessions")], ...sessions.map(s => [s, s])].map(([k, l]) => (
        <RadioRow key={k} on={f.session === k} label={l}
          onClick={() => setF(v => ({ ...v, session: k }))} />
      ))}
      <SectionTitle>{t("overlays:filter.timeRange")}</SectionTitle>
      {[["all", t("overlays:filter.allTime")], ["today", t("overlays:filter.today")],
        ["yesterday", t("overlays:filter.yesterday")], ["earlier", t("overlays:filter.earlier")]].map(([k, l]) => (
        <RadioRow key={k} on={f.time === k} label={l}
          onClick={() => setF(v => ({ ...v, time: k }))} />
      ))}
    </PopShell>
  );
}

// ---------- 快速上手引导(coach marks) ----------
const GUIDE_STEPS = [
  { target: "rail", side: "right", titleKey: "onboarding:guide.step1Title", bodyKey: "onboarding:guide.step1Body" },
  { target: "search", side: "right", titleKey: "onboarding:guide.step2Title", bodyKey: "onboarding:guide.step2Body" },
  { target: "scope", side: "top", scroll: true, titleKey: "onboarding:guide.step3Title", bodyKey: "onboarding:guide.step3Body" },
];
const GUIDE_TOTAL = GUIDE_STEPS.length;

export function Guide({ step, onGo, onFinish }) {
  const { t } = useTranslation();
  const [box, setBox] = useState(null);
  const [card, setCard] = useState(null);
  const cfg = GUIDE_STEPS[step - 1];

  useEffect(() => {
    setBox(null);
    const root = document.querySelector("[data-ferry-win]");
    if (!root || !cfg) return;
    const run = () => {
      const el = document.querySelector(`[data-guide="${cfg.target}"]`);
      if (!el) return;
      const w = root.getBoundingClientRect(), r = el.getBoundingClientRect(), pad = 8;
      const W = w.width, H = w.height, cardW = 324;
      const bl = r.left - w.left - pad, bt = Math.max(8, r.top - w.top - pad);
      const bw = r.width + pad * 2, bh = r.height + pad * 2;
      let cl, ct;
      if (cfg.side === "right") { cl = bl + bw + 18; ct = bt; }
      else if (cfg.side === "top") { cl = bl; ct = bt - 198; }
      else { cl = bl + bw - cardW; ct = bt + bh + 16; }
      cl = Math.min(Math.max(12, cl), W - cardW - 12);
      ct = Math.min(Math.max(12, ct), H - 212);
      setBox({ l: bl, t: bt, w: bw, h: bh, W, H });
      setCard({ left: cl, top: ct });
    };
    // 目标在详情区滚动容器内时,先把它滚到视野中段再量位置
    let delay = 30;
    if (cfg.scroll) {
      const sc = document.querySelector("[data-guide-scroll]");
      const el = document.querySelector(`[data-guide="${cfg.target}"]`);
      if (sc && el) {
        const er = el.getBoundingClientRect(), sr = sc.getBoundingClientRect();
        sc.scrollTop += (er.top - sr.top) - 170;
        delay = 80;
      }
    }
    const t = setTimeout(run, delay);
    return () => clearTimeout(t);
  }, [step]);

  if (!cfg) return null;
  const b = box || { l: -9999, t: 0, w: 0, h: 0, W: 4000, H: 3000 };
  const dim = "var(--dim)";
  return (
    <div style={{ position: "absolute", inset: 0, zIndex: 50 }}>
      {box && (
        <>
          <div style={{ position: "absolute", left: 0, top: 0, width: b.W, height: b.t, background: dim }} />
          <div style={{ position: "absolute", left: 0, top: b.t + b.h, width: b.W,
            height: Math.max(0, b.H - b.t - b.h), background: dim }} />
          <div style={{ position: "absolute", left: 0, top: b.t, width: Math.max(0, b.l),
            height: b.h, background: dim }} />
          <div style={{ position: "absolute", left: b.l + b.w, top: b.t,
            width: Math.max(0, b.W - b.l - b.w), height: b.h, background: dim }} />
          <div style={{ position: "absolute", left: b.l, top: b.t, width: b.w, height: b.h,
            borderRadius: 9, outline: `2px solid ${ACCENT}`,
            boxShadow: "0 0 0 4px var(--ring)", pointerEvents: "none",
            transition: "all .26s cubic-bezier(.2,.7,.3,1)" }} />
        </>
      )}
      <div style={{ position: "absolute", left: card?.left ?? -9999, top: card?.top ?? 0, width: 324,
        background: "var(--bg)", borderRadius: 11,
        boxShadow: "0 18px 44px -16px rgba(20,28,38,.5),0 0 0 1px var(--ring)",
        padding: "16px 18px 14px", transition: "all .26s cubic-bezier(.2,.7,.3,1)",
        animation: "fslide .16s ease" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ fontSize: 11, fontWeight: 700, color: ACCENT, letterSpacing: ".03em" }}>
            {step} / {GUIDE_TOTAL}</span>
          <div style={{ display: "flex", gap: 4, marginLeft: 2 }}>
            {GUIDE_STEPS.map((_, idx) => idx + 1).map(i => (
              <span key={i} style={{ width: 16, height: 3, borderRadius: 2,
                background: i <= step ? ACCENT : "var(--dots)" }} />))}
          </div>
          <span style={{ flex: 1 }} />
          <a onClick={onFinish} style={{ fontSize: 11.5, color: "var(--tx5)" }}>{t("onboarding:guide.skip")}</a>
        </div>
        <div style={{ fontSize: 14.5, fontWeight: 650, marginTop: 11, letterSpacing: "-.01em" }}>{t(cfg.titleKey)}</div>
        <div style={{ fontSize: 12.5, color: "var(--tx3)", lineHeight: 1.55, marginTop: 6 }}>{t(cfg.bodyKey)}</div>
        <div style={{ display: "flex", alignItems: "center", gap: 9, marginTop: 15 }}>
          {step > 1 && (
            <button className="fbtn" style={{ height: 31, fontSize: 12.5 }}
              onClick={() => onGo(step - 1)}>{t("onboarding:guide.back")}</button>)}
          <span style={{ flex: 1 }} />
          <button className="fbtn-primary" style={{ height: 31, padding: "0 16px", fontSize: 12.5 }}
            onClick={() => step >= GUIDE_TOTAL ? onFinish() : onGo(step + 1)}>
            {step >= GUIDE_TOTAL ? t("onboarding:guide.start") : t("onboarding:guide.next")}</button>
        </div>
      </div>
    </div>
  );
}
