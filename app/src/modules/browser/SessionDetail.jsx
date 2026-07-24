// 会话详情:头部 + 会话树 chips + 按轮时间线;轮次操作 hover 显现,有暂存操作时底部浮出操作条
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { supportsAssistantReplyEditing, supportsSessionEditing,
  TOOL_NAME, resumeDescriptor, TOOLS } from "../../shared/contracts/tools.js";
import { ACCENT, fmtSize } from "../../shared/ui/toolDisplay.js";
import { fmtTime, sessionRef, toRounds, toTimeline } from "./sessionModel.js";
import { writeClipboardText } from "../../platform/desktop/client.js";
import { BookmarkIcon, Caret, CheckIcon, CloseIcon, CopyIcon, ImageGlyph, MigrateIcon,
  PencilIcon, RefreshIcon, Spinner, TerminalIcon, ToolIcon, TrashIcon, UndoIcon } from "../../shared/ui/icons.jsx";
import Markdown from "../../shared/ui/Markdown.jsx";
import AssistantReplyEditor from "./AssistantReplyEditor.jsx";
import SessionImagePreview from "./SessionImagePreview.jsx";

const BIG_OUT = 4096;   // 超过此长度的工具输出标记为「大输出」
const LONG_TEXT = 800;  // 超过此长度(或行数)的消息默认折叠
const LONG_LINES = 12;
const FOLD_MAX_H = 250;

const withoutImagePlaceholders = text => String(text || "")
  .replace(/\s*\[Image #\d+\]/g, "").replace(/\n{3,}/g, "\n\n").trim();

function IconBtn({ title, danger, accent, onClick, style, children, ...rest }) {
  return (
    <button title={title} onClick={onClick} {...rest}
      className={`ficon-btn${danger ? " danger" : ""}${accent ? " accent" : ""}`}
      style={style}>{children}</button>
  );
}

// 过长的消息默认折叠,点击展开;fade 为折叠时渐隐遮罩贴合的背景色
function Foldable({ text, fade, children }) {
  const { t: tt } = useTranslation();
  const [open, setOpen] = useState(false);
  const long = text.length > LONG_TEXT ||
    (text.match(/\n/g)?.length || 0) > LONG_LINES;
  if (!long) return children;
  return (
    <>
      <div style={{ position: "relative", overflow: "hidden",
        maxHeight: open ? undefined : FOLD_MAX_H }}>
        {children}
        {!open && (
          <div style={{ position: "absolute", left: 0, right: 0, bottom: 0, height: 52,
            pointerEvents: "none", background: `linear-gradient(to bottom, transparent, ${fade})` }} />
        )}
      </div>
      <button type="button" onClick={e => { e.stopPropagation(); setOpen(v => !v); }}
        style={{ display: "inline-flex", alignItems: "center", gap: 5, marginTop: 6, padding: 0,
          border: "none", background: "transparent", color: "var(--tx4)", fontFamily: "inherit",
          fontSize: 11, fontWeight: 600, cursor: "default" }}>
        <Caret open={open} size={9} />
        {open ? tt("browser:round.collapse") : tt("browser:round.expand", { n: text.length })}
      </button>
    </>
  );
}

function ContextStatusChip({ context }) {
  const { t: tt } = useTranslation();
  if (!context || context.state === "full") return null;
  const summaryKey = context.summary_status === "available"
    ? "summaryAvailable"
    : context.summary_status === "protected"
      ? "summaryProtected" : "summaryMissing";
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 5,
      padding: "2px 7px", borderRadius: 6, color: "var(--warn-text)",
      background: "var(--warn-bg)", border: "1px solid var(--warn-line)" }}>
      {context.state === "in_progress"
        ? tt("browser:context.inProgress")
        : tt("browser:context.compactedCount", { n: context.compaction_count })}
      {context.state !== "in_progress" && <> · {tt(`browser:context.${summaryKey}`)}</>}
    </span>
  );
}

function CompactionBoundary({ compaction }) {
  const { t: tt } = useTranslation();
  const [open, setOpen] = useState(false);
  const summary = compaction.summary || {};
  const readable = summary.status === "available" && !!summary.text;
  const trigger = compaction.trigger === "automatic"
    ? tt("browser:context.automatic")
    : compaction.trigger === "manual"
      ? tt("browser:context.manual") : tt("browser:context.triggerUnknown");
  const status = compaction.state === "in_progress"
    ? tt("browser:context.inProgress")
    : summary.status === "available"
      ? tt("browser:context.summaryAvailable")
      : summary.status === "protected"
        ? tt("browser:context.summaryProtected")
        : tt("browser:context.summaryMissing");
  const metrics = compaction.metrics || {};
  return (
    <div style={{ margin: "18px 0 24px" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <span style={{ flex: 1, height: 1, background: "var(--warn-line)" }} />
        <span style={{ padding: "3px 9px", borderRadius: 999,
          border: "1px solid var(--warn-line)", color: "var(--warn-text)",
          background: "var(--warn-bg)", fontSize: 11, fontWeight: 650 }}>
          {tt("browser:context.boundaryTitle")} · {trigger}
        </span>
        <span style={{ flex: 1, height: 1, background: "var(--warn-line)" }} />
      </div>
      <div style={{ marginTop: 9, padding: "11px 13px", borderRadius: 9,
        border: "1px solid var(--warn-line)", background: "var(--surface)",
        color: "var(--tx3b)", fontSize: 12 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--warn-text)", fontWeight: 650 }}>{status}</span>
          <span style={{ flex: 1 }} />
          {readable && (
            <button type="button" onClick={() => setOpen(value => !value)}
              style={{ border: 0, padding: 0, background: "transparent",
                color: "var(--tx3b)", font: "inherit", cursor: "pointer" }}>
              {open ? tt("browser:context.hideSummary") : tt("browser:context.showSummary")}
            </button>
          )}
        </div>
        <div style={{ marginTop: 5, color: "var(--tx4)", lineHeight: 1.5 }}>
          {tt("browser:context.resumeHint")}
          {compaction.tail?.status === "located" &&
            Number.isInteger(compaction.tail.start_message_index) &&
            <> · {tt("browser:context.tailStartsAt", {
              n: compaction.tail.start_message_index,
            })}</>}
          {Number.isInteger(metrics.pre_tokens) && Number.isInteger(metrics.post_tokens) &&
            <> · {tt("browser:context.tokenChange", {
              before: metrics.pre_tokens.toLocaleString(),
              after: metrics.post_tokens.toLocaleString(),
            })}</>}
        </div>
        {summary.status === "protected" && (
          <div style={{ marginTop: 6, color: "var(--tx4)" }}>
            {tt("browser:context.protectedHint")}
          </div>
        )}
        {open && readable && (
          <div style={{ marginTop: 11, paddingTop: 11,
            borderTop: "1px solid var(--line5)", color: "var(--tx2)" }}>
            <Markdown text={summary.text} />
          </div>
        )}
      </div>
    </div>
  );
}

function ToolCard({ t, tt, open, onToggle }) {
  const big = (t.size || 0) > BIG_OUT;
  const cmd = typeof t.input === "object"
    ? (t.input.command || t.input.file_path || t.input.pattern ||
       JSON.stringify(t.input).slice(0, 80))
    : String(t.input || "").slice(0, 80);
  const out = t.output || tt("browser:tool.noOutput");
  return (
    <div style={{ margin: "5px 0", border: "1px solid var(--line3)", borderRadius: 8,
      overflow: "hidden", background: "var(--fill)" }}>
      <div onClick={onToggle} style={{ display: "flex", alignItems: "center", gap: 9,
        padding: "7px 11px", cursor: "default" }}>
        <Caret open={open} size={10} />
        <span className="mono" style={{ fontSize: 11, fontWeight: 600, color: "var(--tx2b)" }}>{t.name}</span>
        <span className="mono" style={{ fontSize: 11, color: "var(--tx4)", whiteSpace: "nowrap",
          overflow: "hidden", textOverflow: "ellipsis", flex: 1 }}>{cmd}</span>
        {big && (
          <span style={{ fontSize: 10, color: "var(--warn-deep)", background: "var(--warn-bg)",
            padding: "1px 7px", borderRadius: 20, flex: "none" }}>{tt("browser:tool.bigOutput", { size: fmtSize(t.size) })}</span>
        )}
      </div>
      {open && (
        <pre className="mono fscroll selectable" style={{ margin: 0, padding: "11px 13px",
          fontSize: 11, lineHeight: 1.6, color: "var(--tx2b)", whiteSpace: "pre-wrap",
          maxHeight: 200, overflow: "auto", background: "var(--surface)",
          borderTop: "1px solid var(--line5)" }}>
          {out.slice(0, 200000)}
        </pre>
      )}
    </div>
  );
}

function Round({ r, editable, delOp, rewOp, onDelete, onUndoDelete,
  onRewrite, onUpdateRewrite, onCancelRewrite, migratable,
  replyOp, canEditReply, replyEditBlocked, onStartReply, onUpdateReply, onCancelReply,
  scopeOn, onScope, onClearScope, onMigrateScope, scopeStats, onOpenImages }) {
  const { t: tt } = useTranslation();
  const [open, setOpen] = useState({});
  const [toolsOpen, setToolsOpen] = useState(false);
  const [rewEditing, setRewEditing] = useState(false);
  const [copied, setCopied] = useState(false);
  const taRef = useRef(null);
  const userText = useMemo(() => withoutImagePlaceholders(r.user), [r.user]);
  const shownUserText = rewOp ? withoutImagePlaceholders(rewOp.text) : userText;
  const images = r.images || [];
  const fullAiText = r.final || "";
  const aiText = fullAiText.slice(0, 8000);
  const deleted = !!delOp;

  const copyAi = async () => {
    try {
      await writeClipboardText(fullAiText);
      setCopied(true); setTimeout(() => setCopied(false), 1400);
    } catch {}
  };
  const fitTa = el => {
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.max(el.scrollHeight, 48)}px`;
  };
  const startRewrite = () => {
    onRewrite();
    setRewEditing(true);
    setTimeout(() => { const el = taRef.current; if (el) { fitTa(el); el.focus(); } }, 0);
  };

  return (
    <div className="fround" data-round={r.n} style={{ marginBottom: editable ? 10 : 30 }}>
      {editable && (
        <div className={deleted || rewOp ? undefined : "fhact"}
          style={{ display: "flex", alignItems: "center", gap: 9, margin: "10px 0 8px" }}>
          <span style={{ width: 20, height: 20, flex: "none", borderRadius: "50%",
            border: "1.5px solid var(--line2)", display: "inline-flex", alignItems: "center",
            justifyContent: "center", fontSize: 10, fontWeight: 700, color: "var(--tx4b)" }}>{r.n}</span>
          <span style={{ flex: 1, height: 1, background: "var(--hairline)" }} />
          <div style={{ display: "flex", gap: 3 }}>
            {deleted ? (
              <IconBtn title={tt("browser:round.undoDelete")} onClick={onUndoDelete}><UndoIcon /></IconBtn>
            ) : (
              <IconBtn title={tt("browser:round.deleteTurn", { n: r.n })} danger onClick={onDelete}><TrashIcon /></IconBtn>
            )}
            {r.locator && !deleted &&
              <IconBtn title={tt("browser:round.rewriteUser")} onClick={startRewrite}><PencilIcon /></IconBtn>}
          </div>
        </div>
      )}
      <div className={deleted ? "fdel" : undefined}>
        {images.length > 0 && (
          <div style={{ display: "flex", justifyContent: "flex-end", margin: "6px 0 4px" }}>
            <button type="button" title={tt("browser:round.openImages", { n: images.length })} onClick={() => onOpenImages(images)}
              style={{ display: "inline-flex", alignItems: "center",
              gap: 6, padding: "4px 10px", borderRadius: 20, background: "var(--chip)",
              color: "var(--tx3b)", fontSize: 11, fontWeight: 600, border: "1px solid var(--line4)",
              cursor: "pointer", boxShadow: "0 1px 0 rgba(255, 255, 255, .08)" }}>
              <ImageGlyph /> {tt("browser:round.viewImages", { n: images.length })}</button>
          </div>
        )}
        {(shownUserText || (rewOp && rewEditing)) && (
          <div style={{ display: "flex", justifyContent: "flex-end", margin: "6px 0" }}>
            {rewOp && rewEditing && !deleted ? (
              <div style={{ maxWidth: "82%", width: "82%", position: "relative" }}>
                <textarea ref={el => { taRef.current = el; if (el) fitTa(el); }}
                  className="fscroll selectable" value={rewOp.text}
                  onChange={e => { onUpdateRewrite(e.target.value); fitTa(e.target); }}
                  onKeyDown={e => {
                    if (e.key === "Escape") { e.preventDefault(); onCancelRewrite(); setRewEditing(false); }
                    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                      e.preventDefault(); setRewEditing(false);
                    }
                  }}
                  style={{ width: "100%", display: "block", resize: "none", overflow: "hidden",
                    boxSizing: "border-box", background: "var(--fill4)", color: "var(--tx1b)",
                    border: `1.5px solid ${ACCENT}`, padding: "9px 14px", borderRadius: 16,
                    fontSize: 13, lineHeight: 1.65, userSelect: "text",
                    fontFamily: "inherit", whiteSpace: "pre-wrap", overflowWrap: "break-word" }} />
                <div style={{ display: "flex", justifyContent: "flex-end", gap: 3, marginTop: 6 }}>
                  <IconBtn title={tt("browser:round.cancelRewrite")} onClick={() => { onCancelRewrite(); setRewEditing(false); }}>
                    <CloseIcon /></IconBtn>
                  <IconBtn title={tt("browser:round.confirmRewrite")} accent onClick={() => setRewEditing(false)}>
                    <CheckIcon /></IconBtn>
                </div>
              </div>
            ) : (
              <div className="fdel-text selectable" onClick={rewOp && !deleted ? startRewrite : undefined}
                title={rewOp && !deleted ? tt("browser:round.clickToEdit") : undefined}
                style={{ maxWidth: "82%", background: "var(--fill4)", color: "var(--tx1b)",
                  padding: "9px 14px", borderRadius: 16, fontSize: 13, lineHeight: 1.65,
                  overflowWrap: "break-word",
                  cursor: rewOp && !deleted ? "text" : undefined }}>
                <Foldable text={shownUserText} fade="var(--fill4)">
                  <div style={{ whiteSpace: "pre-wrap" }}>{shownUserText.slice(0, 4000)}</div>
                </Foldable>
                {rewOp && !deleted && (
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 4, marginLeft: 8,
                    color: ACCENT, fontSize: 10, fontWeight: 600, whiteSpace: "nowrap" }}>
                    <PencilIcon size={10} /> {tt("browser:round.rewritten")}</span>
                )}
              </div>
            )}
          </div>
        )}
        {!replyOp && r.steps.length > 0 && (
          <div style={{ margin: "8px 0" }}>
            <div onClick={() => setToolsOpen(v => !v)} style={{ display: "inline-flex",
              alignItems: "center", gap: 6, padding: "3px 8px 3px 4px", borderRadius: 6,
              cursor: "default", color: "var(--tx4)", fontSize: 12 }} className="hov-ghost">
              <Caret open={toolsOpen} size={9} />
              <span>{tt("browser:tool.stepCount", { n: r.steps.length })}</span>
            </div>
            {toolsOpen && (
              <div style={{ marginLeft: 18, marginTop: 2, borderLeft: "2px solid var(--line5)",
                paddingLeft: 13 }}>
                {r.steps.map((s, i) => s.kind === "text" ? (
                  <div key={i} className="selectable" style={{ margin: "7px 0", fontSize: 12,
                    lineHeight: 1.65, color: "var(--tx3b)", whiteSpace: "pre-wrap",
                    overflowWrap: "break-word" }}>{s.text.slice(0, 4000)}</div>
                ) : (
                  <ToolCard key={i} t={s.tool} tt={tt} open={open[i] ?? false}
                    onToggle={() => setOpen(o => ({ ...o, [i]: !(o[i] ?? false) }))} />
                ))}
              </div>
            )}
          </div>
        )}
        {!replyOp && (aiText || (canEditReply && !deleted)) && (
          <div style={{ margin: aiText ? "10px 0 0" : "6px 0 0" }}>
            {aiText && <div className="fdel-text"><Markdown text={aiText} /></div>}
            <div className="fhact" style={{ display: "flex", gap: 3, marginTop: 4 }}>
              {aiText && (
                <IconBtn title={copied ? tt("browser:round.copiedAi") : tt("browser:round.copyAi")} onClick={copyAi}>
                  {copied ? <CheckIcon /> : <CopyIcon />}</IconBtn>
              )}
              {canEditReply && !deleted && (
                <IconBtn onClick={onStartReply} disabled={replyEditBlocked}
                  title={replyEditBlocked ? tt("browser:replyEditor.blockedHint") : tt("browser:replyEditor.startHint")}>
                  <PencilIcon /></IconBtn>
              )}
            </div>
          </div>
        )}
        {!deleted && replyOp && (
          <div style={{ marginTop: 7 }}>
            <AssistantReplyEditor op={replyOp}
              onChange={onUpdateReply} onCancel={onCancelReply} />
          </div>
        )}
      </div>
      {migratable && (
        <div style={{ margin: "10px 0 0" }}>
          {!scopeOn ? (
            <div className={r.n === 1 ? undefined : "fhact"}
              style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ flex: 1, height: 1, background: "var(--hairline)" }} />
              <button data-guide={r.n === 1 ? "scope" : undefined}
                className="ficon-btn accent" onClick={onScope}
                style={{ width: "auto", padding: "0 11px", gap: 6, fontSize: 11, fontWeight: 500,
                  border: "1px dashed var(--acc-line2)", borderRadius: 13 }}>
                <BookmarkIcon /> {tt("browser:round.scopeHere")}</button>
              <span style={{ flex: 1, height: 1, background: "var(--hairline)" }} />
            </div>
          ) : (
            <div style={{ border: "1px solid var(--acc-line2)", background: "var(--acc-soft5)", borderRadius: 8,
              padding: "11px 13px" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ fontWeight: 600, color: "var(--acc-text)", fontSize: 12 }}>{tt("browser:round.scopeOnly", { n: r.n })}</span>
                <IconBtn title={tt("browser:round.cancel")} onClick={onClearScope} style={{ marginLeft: "auto" }}><CloseIcon /></IconBtn>
              </div>
              <div style={{ fontSize: 12, color: "var(--tx2b)", marginTop: 5 }}>{scopeStats}</div>
              <button className="fbtn-primary" onClick={onMigrateScope}
                style={{ marginTop: 9, height: 28, padding: "0 13px", fontSize: 12 }}>{tt("browser:round.migrateWithScope")}</button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function PendingBar({ ops, removeOp, onOpenDiff, onApply, applying, invalid, onDiscardAll }) {
  const { t: tt } = useTranslation();
  const [listOpen, setListOpen] = useState(false);
  const jump = n => document.querySelector(`[data-round="${n}"]`)
    ?.scrollIntoView({ behavior: "smooth", block: "center" });
  return (
    <div style={{ position: "absolute", bottom: 20, left: "50%", transform: "translateX(-50%)",
      zIndex: 5 }}>
      {listOpen && (
        <div className="fscroll" style={{ position: "absolute", bottom: "calc(100% + 8px)", left: 0,
          minWidth: 250, maxHeight: 262, overflowY: "auto", background: "var(--bg)",
          border: "1px solid var(--line3)", borderRadius: 10,
          boxShadow: "var(--shadow-menu)", padding: 5 }}>
          {ops.map(o => (
            <div key={o.id} className="hov-ghost" style={{ display: "flex", alignItems: "center",
              gap: 8, padding: "5px 4px 5px 9px", borderRadius: 6 }}>
              <span style={{ width: 6, height: 6, borderRadius: "50%", background: o.dot, flex: "none" }} />
              <a onClick={() => jump(o.n)} style={{ flex: 1, fontSize: 12, color: "var(--tx2)",
                cursor: "default", whiteSpace: "nowrap" }}>{o.labelKey ? tt(o.labelKey, o.labelParams) : o.label}</a>
              <IconBtn title={tt("browser:pendingBar.undoOp")} onClick={() => removeOp(o.id)}><CloseIcon size={11} /></IconBtn>
            </div>
          ))}
        </div>
      )}
      <div style={{ display: "flex", alignItems: "center", gap: 7, padding: 7,
        background: "var(--bg)", border: "1px solid var(--line3)", borderRadius: 24,
        boxShadow: "var(--shadow-sheet)" }}>
        <button className="fbtn" style={{ height: 28, fontSize: 12, borderRadius: 18, fontWeight: 600 }}
          onClick={() => setListOpen(v => !v)}>
          {tt("browser:pendingBar.pendingCount", { n: ops.length })} <Caret open={listOpen} size={9} /></button>
        <button className="fbtn" style={{ height: 28, fontSize: 12, borderRadius: 18 }}
          disabled={!!invalid} title={invalid || undefined} onClick={onOpenDiff}>{tt("browser:pendingBar.previewDiff")}</button>
        <button className="fbtn-primary" style={{ height: 28, fontSize: 12, padding: "0 14px",
          borderRadius: 18 }} disabled={applying || !!invalid} title={invalid || undefined} onClick={onApply}>
          {applying ? tt("browser:pendingBar.applying") : tt("browser:pendingBar.applyChanges")}</button>
        <button className="fbtn" style={{ height: 28, fontSize: 12, borderRadius: 18,
          color: "var(--tx4)" }} onClick={onDiscardAll}>{tt("browser:pendingBar.discard")}</button>
      </div>
      {invalid && <div style={{ position: "absolute", right: 14, bottom: "calc(100% + 5px)",
        maxWidth: 360, padding: "5px 9px", borderRadius: 6, background: "var(--err-bg2)",
        color: "var(--err-text)", fontSize: 11 }}>{invalid}</div>}
    </div>
  );
}

// memo:侧边栏展开/折叠、悬停等与详情无关的状态变化不再重渲染整条时间线
export default memo(function SessionDetail({ meta, data, error,
  scope, setScope, ops, dirtyOps, addOp, removeOp, updateOp,
  startReplyEdit, replyEditError, onOpenDiff, onApply, applying, onDiscardAll,
  onOpenMigrate, onRefresh, refreshing, onResume,
  navigationTarget }) {
  const { t: tt } = useTranslation();
  const rounds = useMemo(() => toRounds(data?.messages, data?.turns), [data]);
  const timeline = useMemo(
    () => toTimeline(rounds, data?.context_compactions),
    [rounds, data?.context_compactions],
  );
  const canEdit = supportsSessionEditing(meta.tool);
  const canEditReply = supportsAssistantReplyEditing(meta.tool);
  const canMigrate = TOOLS.includes(meta.tool);
  const [copied, setCopied] = useState(false);
  const [resuming, setResuming] = useState(false);
  const [previewImages, setPreviewImages] = useState(null);

  useEffect(() => {
    if (!data || navigationTarget?.view !== "library") return;
    const round = Number(navigationTarget.turn);
    if (!Number.isFinite(round) || round < 1) return;
    requestAnimationFrame(() => document.querySelector(`[data-round="${round}"]`)
      ?.scrollIntoView({ behavior: "smooth", block: "center" }));
  }, [data, navigationTarget]);

  const roundSize = r => (r.user?.length || 0) + r.ai.join("").length +
    r.tools.reduce((a, t) => a + (t.size || 0), 0);

  const copyResume = () => {
    resumeDescriptor(meta.tool, sessionRef(meta))
      .then(d => writeClipboardText(d.display_command))
      .catch(() => {});
    setCopied(true); setTimeout(() => setCopied(false), 1600);
  };

  const resumeInTerminal = async () => {
    if (resuming) return;
    setResuming(true);
    try { await onResume(meta); }
    finally { setResuming(false); }
  };

  const scopeMsgs = scope
    ? (rounds.slice(0, scope).reduce((a, r) => a + 1 + (r.ai.length ? 1 : 0), 0) +
       rounds.slice(0, scope).reduce((a, r) => a + r.tools.length, 0))
    : 0;
  const scopeStats = scope
    ? tt("browser:round.scopeStats", { msgs: scopeMsgs, size: fmtSize(rounds.slice(0, scope).reduce((a, r) => a + roundSize(r), 0)) })
    : "";

  const opFor = (n, type) => ops.find(o => o.type === type && o.n === n);

  const subCount = data ? data.tree_count - 1 : 0;

  return (
    <div style={{ flex: 1, display: "flex", minWidth: 0, minHeight: 0, position: "relative" }}>
      <div className="fscroll" data-guide-scroll="1"
        style={{ flex: 1, overflowY: "auto", minWidth: 0 }}>
        <div style={{ padding: "18px 26px 14px", borderBottom: "1px solid var(--line5)", position: "sticky",
          top: 0, background: "var(--veil)", backdropFilter: "blur(6px)", zIndex: 2 }}>
          <div style={{ display: "flex", alignItems: "flex-start", gap: 13 }}>
            <ToolIcon tool={meta.tool} size={40} dot="var(--ok)" />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 16, fontWeight: 650, letterSpacing: "-.01em" }}>
                {meta.title || tt("browser:session.untitled")}</div>
              <div style={{ display: "flex", flexWrap: "wrap", gap: "6px 14px", marginTop: 6,
                fontSize: 12, color: "var(--tx3b)" }}>
                <span>{tt("browser:session.source")} <b style={{ color: "var(--tx2)", fontWeight: 600 }}>{TOOL_NAME[meta.tool]}</b></span>
                <span className="mono" style={{ color: "var(--tx4)" }}>{meta.dir}</span>
                <span>{tt("browser:session.messages", { n: data ? data.count : meta.count })}</span>
                <ContextStatusChip context={data?.context} />
                <span>{fmtSize(meta.size)}</span>
                <span>{tt("browser:session.active", { time: fmtTime(meta.updated, tt) })}</span>
              </div>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 2, flex: "none" }}>
              <button className="ftool-btn" title={tt("browser:session.refresh")}
                disabled={refreshing} onClick={onRefresh}>
                {refreshing ? <Spinner size={14} /> : <RefreshIcon />}</button>
              <button className="ftool-btn" onClick={resumeInTerminal} disabled={resuming}
                title={resuming ? tt("browser:session.resumingTerminal") : tt("browser:session.resumeTerminal")}>
                {resuming ? <Spinner size={14} /> : <TerminalIcon />}</button>
              <button className="ftool-btn" onClick={copyResume}
                title={copied ? tt("browser:session.copiedResume") : tt("browser:session.copyResume")}
                style={copied ? { color: "var(--ok)" } : undefined}>
                {copied ? <CheckIcon size={15} /> : <CopyIcon size={15} />}</button>
              {canMigrate && (
                <button data-guide="migrate" className="ftool-btn"
                  title={tt("browser:session.migrate")}
                  onClick={() => onOpenMigrate(null)}><MigrateIcon /></button>
              )}
            </div>
          </div>
          {subCount > 0 && (
            <div className="mono" style={{ display: "flex", alignItems: "center", gap: 7, marginTop: 13,
              fontSize: 11, color: "var(--tx4)" }}>
              <span style={{ padding: "2px 8px", borderRadius: 6, background: "var(--chip)",
                color: "var(--tx3b)" }}>{tt("browser:session.subSessionsLine", { tool: TOOL_NAME[meta.tool] })}</span>
              <span>{tt("browser:session.arrow")}</span>
              <span style={{ padding: "2px 8px", borderRadius: 6, color: "var(--tx3b)" }}>
                {tt("browser:session.subSessions", { n: subCount })}</span>
            </div>
          )}
        </div>
        <div style={{ padding: `20px 26px ${dirtyOps.length ? 110 : 48}px`, maxWidth: 720, margin: "0 auto" }}>
          {error && <div style={{ padding: 30, color: "var(--err-deep)", fontSize: 13 }}>{tt("browser:session.readFailed", { error })}</div>}
          {!data && !error && (
            <div style={{ padding: 40, display: "flex", alignItems: "center", gap: 10,
              color: "var(--tx4)", fontSize: 13 }}><Spinner size={16} /> {tt("browser:session.parsing")}</div>
          )}
          {data && timeline.map(item => {
            if (item.kind === "compaction") {
              return <CompactionBoundary key={item.key} compaction={item.compaction} />;
            }
            const r = item.round;
            return (
            <Round key={item.key} r={r} editable={canEdit}
              delOp={opFor(r.n, "delete")} rewOp={opFor(r.n, "rewrite")}
              replyOp={opFor(r.n, "assistant-reply")}
              canEditReply={canEditReply && !!r.assistantReply}
              replyEditBlocked={ops.length > 0 && !opFor(r.n, "assistant-reply")}
              onDelete={() => addOp("delete", r)}
              onUndoDelete={() => { const o = opFor(r.n, "delete"); if (o) removeOp(o.id); }}
              onRewrite={() => addOp("rewrite", r)}
              onUpdateRewrite={text => { const o = opFor(r.n, "rewrite"); if (o) updateOp(o.id, { text }); }}
              onCancelRewrite={() => { const o = opFor(r.n, "rewrite"); if (o) removeOp(o.id); }}
              onStartReply={() => startReplyEdit(r.assistantReply)}
              onUpdateReply={items => { const o = opFor(r.n, "assistant-reply");
                if (o) updateOp(o.id, { items }); }}
              onCancelReply={() => { const o = opFor(r.n, "assistant-reply"); if (o) removeOp(o.id); }}
              migratable={canMigrate && r.n < rounds.length}
              scopeOn={scope === r.n}
              onScope={() => setScope(r.n)}
               onClearScope={() => setScope(null)}
               onMigrateScope={() => onOpenMigrate(r.n)}
               scopeStats={scopeStats} onOpenImages={setPreviewImages} />
            );
          })}
          {data && rounds.length === 0 && (
            <div style={{ padding: 30, color: "var(--tx5)", fontSize: 12 }}>{tt("browser:session.noMessages")}</div>
          )}
        </div>
      </div>
      {dirtyOps.length > 0 && (
        <PendingBar ops={dirtyOps} removeOp={removeOp}
          onOpenDiff={onOpenDiff} onApply={onApply} applying={applying}
          invalid={replyEditError(dirtyOps.find(op => op.type === "assistant-reply"))}
          onDiscardAll={onDiscardAll} />
      )}
      {previewImages && <SessionImagePreview key={previewImages[0]?.id} images={previewImages}
        meta={meta} onClose={() => setPreviewImages(null)} />}
    </div>
  );
});
