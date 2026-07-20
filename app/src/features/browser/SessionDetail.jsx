// 会话详情:头部 + 会话树 chips + 按轮时间线;轮次操作 hover 显现,有暂存操作时底部浮出操作条
import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOL_NAME } from "../../api/contract/tools.js";
import { resumeDescriptor } from "../../api/contract/tools.js";
import { ACCENT, fmtSize } from "../../domain/tools/toolDisplay.js";
import { fmtTime, toRounds } from "../../domain/sessions/sessionModel.js";
import { BookmarkIcon, Caret, CheckIcon, CloseIcon, CopyIcon, ImageGlyph,
  PencilIcon, Spinner, ToolIcon, TrashIcon, UndoIcon } from "../../components/ui/icons.jsx";
import Markdown from "../../components/ui/Markdown.jsx";
import AssistantReplyEditor from "./AssistantReplyEditor.jsx";

const BIG_OUT = 4096;   // 超过此长度的工具输出标记为「大输出」

// 用户消息里的图片占位(粘贴图片的缓存路径)不直出,收成计数
const IMG_RE = /\[Image:\s*source:[^\]]*\]|\[Image #\d+\]/g;
const splitImages = text => {
  const imgs = (String(text || "").match(IMG_RE) || []).length;
  return { text: String(text || "").replace(IMG_RE, "").replace(/\n{3,}/g, "\n\n").trim(), imgs };
};

function IconBtn({ title, danger, accent, onClick, style, children, ...rest }) {
  return (
    <button title={title} onClick={onClick} {...rest}
      className={`ficon-btn${danger ? " danger" : ""}${accent ? " accent" : ""}`}
      style={style}>{children}</button>
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
        padding: "7px 11px", cursor: "pointer" }}>
        <Caret open={open} size={10} />
        <span className="mono" style={{ fontSize: 11.5, fontWeight: 600, color: "var(--tx2b)" }}>{t.name}</span>
        <span className="mono" style={{ fontSize: 11.5, color: "var(--tx4)", whiteSpace: "nowrap",
          overflow: "hidden", textOverflow: "ellipsis", flex: 1 }}>{cmd}</span>
        {big && (
          <span style={{ fontSize: 10.5, color: "var(--warn-deep)", background: "var(--warn-bg)",
            padding: "1px 7px", borderRadius: 20, flex: "none" }}>{tt("browser:tool.bigOutput", { size: fmtSize(t.size) })}</span>
        )}
      </div>
      {open && (
        <pre className="mono fscroll selectable" style={{ margin: 0, padding: "11px 13px",
          fontSize: 11.5, lineHeight: 1.6, color: "var(--tx2b)", whiteSpace: "pre-wrap",
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
  replyOp, canAuthor, authoringBlocked, onStartReply, onUpdateReply, onCancelReply,
  scopeOn, onScope, onClearScope, onMigrateScope, scopeStats }) {
  const { t: tt } = useTranslation();
  const [open, setOpen] = useState({});
  const [toolsOpen, setToolsOpen] = useState(false);
  const [rewEditing, setRewEditing] = useState(false);
  const [copied, setCopied] = useState(false);
  const taRef = useRef(null);
  const { text: userText, imgs } = useMemo(() => splitImages(r.user), [r.user]);
  const fullAiText = r.final || "";
  const aiText = fullAiText.slice(0, 8000);
  const deleted = !!delOp;

  const copyAi = () => {
    try { navigator.clipboard?.writeText(fullAiText); } catch {}
    setCopied(true); setTimeout(() => setCopied(false), 1400);
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
            justifyContent: "center", fontSize: 10.5, fontWeight: 700, color: "var(--tx4b)" }}>{r.n}</span>
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
        {r.user && imgs > 0 && (
          <div style={{ display: "flex", justifyContent: "flex-end", margin: "6px 0 4px" }}>
            <span title={tt("browser:round.imagesCount", { n: imgs })} style={{ display: "inline-flex", alignItems: "center",
              gap: 5, padding: "2px 9px", borderRadius: 20, background: "var(--chip)",
              color: "var(--tx4)", fontSize: 10.5 }}>
              <ImageGlyph /> ×{imgs}</span>
          </div>
        )}
        {r.user && (
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
                    fontSize: 13, lineHeight: 1.65, outline: "none", userSelect: "text",
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
                  whiteSpace: "pre-wrap", overflowWrap: "break-word",
                  cursor: rewOp && !deleted ? "text" : undefined }}>
                {(rewOp ? rewOp.text : userText).slice(0, 4000)}
                {rewOp && !deleted && (
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 4, marginLeft: 8,
                    color: ACCENT, fontSize: 10.5, fontWeight: 600, whiteSpace: "nowrap" }}>
                    <PencilIcon size={10} /> {tt("browser:round.rewritten")}</span>
                )}
              </div>
            )}
          </div>
        )}
        {!replyOp && r.steps.length > 0 && (
          <div style={{ margin: "8px 0" }}>
            <div onClick={() => setToolsOpen(v => !v)} style={{ display: "inline-flex",
              alignItems: "center", gap: 6, padding: "3px 8px 3px 4px", borderRadius: 7,
              cursor: "pointer", color: "var(--tx4)", fontSize: 12 }} className="hov-ghost">
              <Caret open={toolsOpen} size={9} />
              <span>{tt("browser:tool.stepCount", { n: r.steps.length })}</span>
            </div>
            {toolsOpen && (
              <div style={{ marginLeft: 18, marginTop: 2, borderLeft: "2px solid var(--line5)",
                paddingLeft: 13 }}>
                {r.steps.map((s, i) => s.kind === "text" ? (
                  <div key={i} className="selectable" style={{ margin: "7px 0", fontSize: 12.5,
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
        {!replyOp && aiText && (
          <div style={{ margin: "10px 0 0" }}>
            <div className="fdel-text"><Markdown text={aiText} /></div>
            <div className="fhact" style={{ marginTop: 4 }}>
              <IconBtn title={copied ? tt("browser:round.copiedAi") : tt("browser:round.copyAi")} onClick={copyAi}>
                {copied ? <CheckIcon /> : <CopyIcon />}</IconBtn>
            </div>
          </div>
        )}
        {!deleted && (
          <div style={{ marginTop: 7 }}>
            <AssistantReplyEditor op={replyOp} canAuthor={canAuthor}
              blocked={authoringBlocked} onStart={onStartReply}
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
                style={{ width: "auto", padding: "0 11px", gap: 6, fontSize: 11.5, fontWeight: 500,
                  border: "1px dashed var(--acc-line2)", borderRadius: 13 }}>
                <BookmarkIcon /> {tt("browser:round.scopeHere")}</button>
              <span style={{ flex: 1, height: 1, background: "var(--hairline)" }} />
            </div>
          ) : (
            <div style={{ border: "1px solid var(--acc-line2)", background: "var(--acc-soft5)", borderRadius: 9,
              padding: "11px 13px" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ fontWeight: 600, color: "var(--acc-text)", fontSize: 12.5 }}>{tt("browser:round.scopeOnly", { n: r.n })}</span>
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
      zIndex: 5, animation: "ffade .16s ease" }}>
      {listOpen && (
        <div className="fscroll" style={{ position: "absolute", bottom: "calc(100% + 8px)", left: 0,
          minWidth: 250, maxHeight: 262, overflowY: "auto", background: "var(--bg)",
          border: "1px solid var(--line3)", borderRadius: 11,
          boxShadow: "0 14px 36px -14px rgba(20,28,38,.45)", padding: 5 }}>
          {ops.map(o => (
            <div key={o.id} className="hov-ghost" style={{ display: "flex", alignItems: "center",
              gap: 8, padding: "5px 4px 5px 9px", borderRadius: 7 }}>
              <span style={{ width: 6, height: 6, borderRadius: "50%", background: o.dot, flex: "none" }} />
              <a onClick={() => jump(o.n)} style={{ flex: 1, fontSize: 12, color: "var(--tx2)",
                cursor: "pointer", whiteSpace: "nowrap" }}>{o.label}</a>
              <IconBtn title={tt("browser:pendingBar.undoOp")} onClick={() => removeOp(o.id)}><CloseIcon size={11} /></IconBtn>
            </div>
          ))}
        </div>
      )}
      <div style={{ display: "flex", alignItems: "center", gap: 7, padding: 7,
        background: "var(--bg)", border: "1px solid var(--line3)", borderRadius: 24,
        boxShadow: "0 14px 40px -14px rgba(20,28,38,.5)" }}>
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
        maxWidth: 360, padding: "5px 9px", borderRadius: 7, background: "var(--err-bg2)",
        color: "var(--err-text)", fontSize: 11.5 }}>{invalid}</div>}
    </div>
  );
}

export default function SessionDetail({ meta, data, error,
  scope, setScope, ops, addOp, removeOp, updateOp,
  startReplyEdit, authoringError, onOpenDiff, onApply, applying, onDiscardAll,
  onOpenMigrate, editCaps, authoringCaps }) {
  const { t: tt } = useTranslation();
  const rounds = useMemo(() => toRounds(data?.messages, data?.turns), [data]);
  const canEdit = !!editCaps && (editCaps.inplace || editCaps.save_as);
  const canAuthor = !!authoringCaps && (authoringCaps.inplace || authoringCaps.save_as);
  const [copied, setCopied] = useState(false);

  const roundSize = r => (r.user?.length || 0) + r.ai.join("").length +
    r.tools.reduce((a, t) => a + (t.size || 0), 0);

  const copyResume = () => {
    resumeDescriptor(meta.tool, meta.id, meta.dir)
      .then(d => navigator.clipboard?.writeText(d.display_command))
      .catch(() => {});
    setCopied(true); setTimeout(() => setCopied(false), 1600);
  };

  const scopeMsgs = scope
    ? (rounds.slice(0, scope).reduce((a, r) => a + 1 + (r.ai.length ? 1 : 0), 0) +
       rounds.slice(0, scope).reduce((a, r) => a + r.tools.length, 0))
    : 0;
  const scopeStats = scope
    ? `约 ${scopeMsgs} 条消息 · ${fmtSize(rounds.slice(0, scope).reduce((a, r) => a + roundSize(r), 0))}`
    : "";

  const opFor = (n, type) => ops.find(o => o.type === type && o.n === n);

  const subCount = data ? data.tree_count - 1 : 0;

  return (
    <div style={{ flex: 1, display: "flex", minWidth: 0, minHeight: 0, position: "relative" }}>
      <div className="fscroll" data-guide-scroll="1"
        style={{ flex: 1, overflowY: "auto", minWidth: 0, animation: "ffade .16s ease" }}>
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
                <span>{fmtSize(meta.size)}</span>
                <span>{tt("browser:session.active", { time: fmtTime(meta.updated, tt) })}</span>
              </div>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 8, flex: "none" }}>
              <button className="fbtn" style={{ height: 30, fontSize: 12.5 }} onClick={copyResume}>
                {copied ? tt("browser:session.copiedResume") : tt("browser:session.copyResume")}</button>
              <button data-guide="migrate" className="fbtn-primary"
                style={{ height: 30, padding: "0 14px" }}
                onClick={() => onOpenMigrate(null)}>{tt("browser:session.migrate")}</button>
            </div>
          </div>
          {subCount > 0 && (
            <div className="mono" style={{ display: "flex", alignItems: "center", gap: 7, marginTop: 13,
              fontSize: 11.5, color: "var(--tx4)" }}>
              <span style={{ padding: "2px 8px", borderRadius: 6, background: "var(--chip)",
                color: "var(--tx3b)" }}>{tt("browser:session.subSessionsLine", { tool: TOOL_NAME[meta.tool] })}</span>
              <span>{tt("browser:session.arrow")}</span>
              <span style={{ padding: "2px 8px", borderRadius: 6, color: "var(--tx3b)" }}>
                {tt("browser:session.subSessions", { n: subCount })}</span>
            </div>
          )}
        </div>
        <div style={{ padding: `20px 26px ${ops.length ? 110 : 48}px`, maxWidth: 720, margin: "0 auto" }}>
          {error && <div style={{ padding: 30, color: "var(--err-deep)", fontSize: 13 }}>{tt("browser:session.readFailed", { error })}</div>}
          {!data && !error && (
            <div style={{ padding: 40, display: "flex", alignItems: "center", gap: 10,
              color: "var(--tx4)", fontSize: 13 }}><Spinner size={16} /> {tt("browser:session.parsing")}</div>
          )}
          {data && rounds.map(r => (
            <Round key={r.n} r={r} editable={canEdit}
              delOp={opFor(r.n, "delete")} rewOp={opFor(r.n, "rewrite")}
              replyOp={opFor(r.n, "assistant-reply")} canAuthor={canAuthor && !!r.authoring}
              authoringBlocked={ops.length > 0 && !opFor(r.n, "assistant-reply")}
              onDelete={() => addOp("delete", r)}
              onUndoDelete={() => { const o = opFor(r.n, "delete"); if (o) removeOp(o.id); }}
              onRewrite={() => addOp("rewrite", r)}
              onUpdateRewrite={text => { const o = opFor(r.n, "rewrite"); if (o) updateOp(o.id, { text }); }}
              onCancelRewrite={() => { const o = opFor(r.n, "rewrite"); if (o) removeOp(o.id); }}
              onStartReply={() => startReplyEdit(r.authoring)}
              onUpdateReply={items => { const o = opFor(r.n, "assistant-reply");
                if (o) updateOp(o.id, { items }); }}
              onCancelReply={() => { const o = opFor(r.n, "assistant-reply"); if (o) removeOp(o.id); }}
              migratable={r.n < rounds.length}
              scopeOn={scope === r.n}
              onScope={() => setScope(r.n)}
              onClearScope={() => setScope(null)}
              onMigrateScope={() => onOpenMigrate(r.n)}
              scopeStats={scopeStats} />
          ))}
          {data && rounds.length === 0 && (
            <div style={{ padding: 30, color: "var(--tx5)", fontSize: 12.5 }}>{tt("browser:session.noMessages")}</div>
          )}
        </div>
      </div>
      {ops.length > 0 && (
        <PendingBar ops={ops} removeOp={removeOp}
          onOpenDiff={onOpenDiff} onApply={onApply} applying={applying}
          invalid={authoringError(ops.find(op => op.type === "assistant-reply"))}
          onDiscardAll={onDiscardAll} />
      )}
    </div>
  );
}
