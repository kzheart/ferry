// 会话详情:头部 + 会话树 chips + 按轮时间线;轮次操作 hover 显现,有暂存操作时底部浮出操作条
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOL_NAME, resumeDescriptor, toolHasCapability } from "../../api/contract/tools.js";
import { ACCENT, fmtSize } from "../../domain/tools/toolDisplay.js";
import { fmtTime, sessionRef, toRounds } from "../../domain/sessions/sessionModel.js";
import { rpc } from "../../api/transport/rpc.js";
import { BookmarkIcon, Caret, CheckIcon, CloseIcon, CopyIcon, ImageGlyph, MigrateIcon,
  PencilIcon, RefreshIcon, Spinner, TerminalIcon, ToolIcon, TrashIcon, UndoIcon } from "../../components/ui/icons.jsx";
import Markdown from "../../components/ui/Markdown.jsx";
import AssistantReplyEditor from "./AssistantReplyEditor.jsx";

const BIG_OUT = 4096;   // 超过此长度的工具输出标记为「大输出」

const withoutImagePlaceholders = text => String(text || "")
  .replace(/\s*\[Image #\d+\]/g, "").replace(/\n{3,}/g, "\n\n").trim();

function ImagePreview({ images, meta, onClose }) {
  const { t: tt } = useTranslation();
  const [selected, setSelected] = useState(0);
  const [sources, setSources] = useState({});
  const [error, setError] = useState("");
  const [contextMenu, setContextMenu] = useState(null);
  const [copied, setCopied] = useState(false);
  const image = images[selected];
  const source = sources[image.id];

  useEffect(() => {
    const closeOnEscape = event => { if (event.key === "Escape") onClose(); };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  useEffect(() => {
    if (source) return;
    let cancelled = false;
    setError("");
    rpc("session_asset", { tool: meta.tool, ref: sessionRef(meta), asset_id: image.id })
      .then(asset => {
        if (!cancelled) setSources(current => ({ ...current,
          [image.id]: `data:${asset.mime_type};base64,${asset.data}` }));
      })
      .catch(() => { if (!cancelled) setError(tt("browser:round.imageLoadFailed")); });
    return () => { cancelled = true; };
  }, [image.id, meta, source, tt]);

  const choose = index => { setSelected(index); setError(""); };
  const previous = () => choose((selected + images.length - 1) % images.length);
  const next = () => choose((selected + 1) % images.length);
  const copyImage = async () => {
    try {
      if (!navigator.clipboard?.write || typeof ClipboardItem === "undefined") throw new Error();
      const blob = await (await fetch(source)).blob();
      await navigator.clipboard.write([new ClipboardItem({ [blob.type]: blob })]);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      setError(tt("browser:round.imageCopyFailed"));
    }
    setContextMenu(null);
  };
  return (
    <div role="dialog" aria-modal="true" aria-label={tt("browser:round.imagePreview")}
      onMouseDown={event => { setContextMenu(null); if (event.target === event.currentTarget) onClose(); }}
      style={{ position: "fixed", inset: 0, zIndex: 20, display: "flex", alignItems: "center",
        justifyContent: "center", padding: 22, background: "rgba(7, 9, 13, .8)", backdropFilter: "blur(9px)" }}>
      <div style={{ width: "min(940px, 100%)", maxHeight: "min(760px, 100%)", display: "flex",
        flexDirection: "column", overflow: "hidden", border: "1px solid var(--line3)", borderRadius: 14,
        background: "var(--bg)", boxShadow: "0 28px 90px rgba(0, 0, 0, .45)" }}>
        <div style={{ height: 48, display: "flex", alignItems: "center", padding: "0 12px 0 16px",
          borderBottom: "1px solid var(--line5)", gap: 10 }}>
          <ImageGlyph size={14} />
          <span style={{ fontSize: 12, fontWeight: 650, color: "var(--tx2)", flex: 1 }}>
            {tt("browser:round.imagePosition", { current: selected + 1, total: images.length })}
          </span>
          {copied && <span style={{ fontSize: 11, color: "var(--ok)", fontWeight: 600 }}>{tt("browser:round.imageCopied")}</span>}
          <IconBtn title={tt("browser:round.closeImagePreview")} onClick={onClose}><CloseIcon /></IconBtn>
        </div>
        <div style={{ minHeight: 220, flex: 1, position: "relative", display: "flex", alignItems: "center",
          justifyContent: "center", padding: 16, overflow: "auto", background: "var(--surface)" }}>
          {!source && !error && <Spinner size={20} />}
          {error && <span style={{ color: "var(--err-text)", fontSize: 12 }}>{error}</span>}
          {source && <img src={source} alt={image.filename || tt("browser:round.imageAlt", { n: selected + 1 })}
            onContextMenu={event => {
              event.preventDefault();
              setContextMenu({ x: Math.min(event.clientX, window.innerWidth - 178),
                y: Math.min(event.clientY, window.innerHeight - 54) });
            }}
            style={{ maxWidth: "100%", maxHeight: "calc(min(760px, 100vh) - 148px)", display: "block",
              objectFit: "contain", borderRadius: 6, boxShadow: "0 6px 24px rgba(0, 0, 0, .2)" }} />}
          {images.length > 1 && <>
            <button type="button" title={tt("browser:round.previousImage")} onClick={previous}
              style={{ position: "absolute", left: 12, top: "50%", transform: "translateY(-50%) rotate(180deg)",
                width: 34, height: 34, display: "inline-flex", alignItems: "center", justifyContent: "center",
                border: "1px solid var(--line3)", borderRadius: "50%", background: "var(--bg)", color: "var(--tx2)", cursor: "default" }}><Caret open={false} size={15} /></button>
            <button type="button" title={tt("browser:round.nextImage")} onClick={next}
              style={{ position: "absolute", right: 12, top: "50%", transform: "translateY(-50%)",
                width: 34, height: 34, display: "inline-flex", alignItems: "center", justifyContent: "center",
                border: "1px solid var(--line3)", borderRadius: "50%", background: "var(--bg)", color: "var(--tx2)", cursor: "default" }}><Caret open={false} size={15} /></button>
          </>}
        </div>
        {images.length > 1 && <div style={{ display: "flex", justifyContent: "center", gap: 6, padding: 10,
          borderTop: "1px solid var(--line5)", overflowX: "auto" }}>
          {images.map((item, index) => <button key={item.id} type="button" onClick={() => choose(index)}
            title={tt("browser:round.imagePosition", { current: index + 1, total: images.length })}
            style={{ width: index === selected ? 18 : 6, height: 6, flex: "none", padding: 0,
              border: "none", borderRadius: 8, background: index === selected ? ACCENT : "var(--line2)",
              cursor: "default", transition: "width .16s ease" }} />)}
        </div>}
      </div>
      {contextMenu && <div role="menu" onMouseDown={event => event.stopPropagation()}
        style={{ position: "fixed", zIndex: 21, left: contextMenu.x, top: contextMenu.y, minWidth: 166,
          padding: 5, border: "1px solid var(--line2)", borderRadius: 10, background: "var(--bg)",
          boxShadow: "0 14px 38px rgba(0, 0, 0, .32)" }}>
        <button role="menuitem" type="button" onClick={copyImage}
          style={{ width: "100%", height: 32, padding: "0 10px", display: "flex", alignItems: "center",
            border: "none", borderRadius: 6, background: "transparent", color: "var(--tx1)",
            fontFamily: "inherit", fontSize: 12, textAlign: "left", cursor: "pointer" }}>
          {tt("browser:round.copyImage")}
        </button>
      </div>}
    </div>
  );
}

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
  replyOp, canAuthor, authoringBlocked, onStartReply, onUpdateReply, onCancelReply,
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
                  whiteSpace: "pre-wrap", overflowWrap: "break-word",
                  cursor: rewOp && !deleted ? "text" : undefined }}>
                {shownUserText.slice(0, 4000)}
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
  scope, setScope, ops, addOp, removeOp, updateOp,
  startReplyEdit, authoringError, onOpenDiff, onApply, applying, onDiscardAll,
  onOpenMigrate, onRefresh, refreshing, editCaps, authoringCaps }) {
  const { t: tt } = useTranslation();
  const rounds = useMemo(() => toRounds(data?.messages, data?.turns), [data]);
  const canEdit = !!editCaps && (editCaps.inplace || editCaps.save_as);
  const canAuthor = !!authoringCaps && (authoringCaps.inplace || authoringCaps.save_as);
  const canMigrate = toolHasCapability(meta.tool, "migrate-source");
  const [copied, setCopied] = useState(false);
  const [previewImages, setPreviewImages] = useState(null);

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
                <span>{fmtSize(meta.size)}</span>
                <span>{tt("browser:session.active", { time: fmtTime(meta.updated, tt) })}</span>
              </div>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 2, flex: "none" }}>
              <button className="ftool-btn" title={tt("browser:session.refresh")}
                disabled={refreshing} onClick={onRefresh}>
                {refreshing ? <Spinner size={14} /> : <RefreshIcon />}</button>
              <button className="ftool-btn" onClick={copyResume}
                title={copied ? tt("browser:session.copiedResume") : tt("browser:session.copyResume")}
                style={copied ? { color: "var(--ok)" } : undefined}>
                {copied ? <CheckIcon size={15} /> : <TerminalIcon />}</button>
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
              migratable={canMigrate && r.n < rounds.length}
              scopeOn={scope === r.n}
              onScope={() => setScope(r.n)}
               onClearScope={() => setScope(null)}
               onMigrateScope={() => onOpenMigrate(r.n)}
               scopeStats={scopeStats} onOpenImages={setPreviewImages} />
          ))}
          {data && rounds.length === 0 && (
            <div style={{ padding: 30, color: "var(--tx5)", fontSize: 12 }}>{tt("browser:session.noMessages")}</div>
          )}
        </div>
      </div>
      {ops.length > 0 && (
        <PendingBar ops={ops} removeOp={removeOp}
          onOpenDiff={onOpenDiff} onApply={onApply} applying={applying}
          invalid={authoringError(ops.find(op => op.type === "assistant-reply"))}
          onDiscardAll={onDiscardAll} />
      )}
      {previewImages && <ImagePreview key={previewImages[0]?.id} images={previewImages}
        meta={meta} onClose={() => setPreviewImages(null)} />}
    </div>
  );
});
