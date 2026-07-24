// 会话详情:头部 + 会话树 chips + 按轮时间线;轮次操作 hover 显现,有暂存操作时底部浮出操作条
import { memo, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { supportsAssistantReplyEditing, supportsSessionEditing,
  TOOL_NAME, resumeDescriptor, TOOLS } from "../../shared/contracts/tools.js";
import { fmtSize } from "../../shared/ui/toolDisplay.js";
import { fmtTime, sessionRef, toRounds, toTimeline } from "./sessionModel.js";
import { writeClipboardText } from "../../platform/desktop/client.js";
import {
  CheckIcon,
  CopyIcon,
  MigrateIcon,
  RefreshIcon,
  Spinner,
  TerminalIcon,
  ToolIcon,
} from "../../shared/ui/icons.jsx";
import PendingEditBar from "./PendingEditBar.jsx";
import {
  CompactionBoundary,
  ContextStatusChip,
} from "./SessionContext.jsx";
import SessionImagePreview from "./SessionImagePreview.jsx";
import SessionRound from "./SessionRound.jsx";

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
            <SessionRound key={item.key} r={r} editable={canEdit}
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
        <PendingEditBar ops={dirtyOps} removeOp={removeOp}
          onOpenDiff={onOpenDiff} onApply={onApply} applying={applying}
          invalid={replyEditError(dirtyOps.find(op => op.type === "assistant-reply"))}
          onDiscardAll={onDiscardAll} />
      )}
      {previewImages && <SessionImagePreview key={previewImages[0]?.id} images={previewImages}
        meta={meta} onClose={() => setPreviewImages(null)} />}
    </div>
  );
});
