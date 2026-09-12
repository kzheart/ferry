import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AutoModeIcon,
  Caret,
  ManualModeIcon,
  ProviderIcon,
  SendArrowIcon,
  StopFillIcon,
  ToolIcon,
} from "../../shared/ui/icons.jsx";
import { useFerryRuntime } from "../../shared/capabilities/ferryRuntime.jsx";
import { repoOf, sessionAttachmentKey } from "../browser/public.js";
import { ModeMenu, ModelMenu } from "./AgentMenus.jsx";
import "./agentComposer.css";

function MentionMenu({ query, sessions, onPick }) {
  const q = query.toLowerCase();
  const matched = useMemo(() => sessions
    .filter(s => !q || (s.title || "").toLowerCase().includes(q)
      || (s.id || "").toLowerCase().includes(q))
    .slice(0, 8), [sessions, q]);
  if (!matched.length) return null;
  return (
    <div style={{ position: "absolute", left: 12, right: 12, bottom: "100%", marginBottom: 8,
      background: "var(--bg)", borderRadius: 11, boxShadow: "var(--shadow-menu)",
      overflow: "hidden", zIndex: 20, padding: 4, animation: "fpop .14s ease" }}>
      {matched.map(s => (
        <div key={s.id} className="hov-item" onMouseDown={e => { e.preventDefault(); onPick(s); }}
          style={{ display: "flex", alignItems: "center", gap: 8, padding: "5px 8px",
            borderRadius: 7, cursor: "default" }}>
          <ToolIcon tool={s.tool} size={18} />
          <span style={{ fontSize: 12, color: "var(--tx1)", flex: 1, minWidth: 0,
            overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {s.title || s.id}</span>
          <span style={{ fontSize: 10.5, color: "var(--tx5)" }}>{repoOf(s.dir)}</span>
        </div>
      ))}
    </div>
  );
}

export function AgentComposer({ text, setTextValue, taRef, mention, scanSessions,
  onPickMention, onKeyDown, onPaste, onSend, onSteer, running, mode, onOpenConfig,
  health, autoFocus, attachments, onRemoveAttachment, activity }) {
  const { t } = useTranslation();
  const ferry = useFerryRuntime();
  const modeAnchorRef = useRef(null);
  const modelAnchorRef = useRef(null);
  const [modeOpen, setModeOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const hasContent = !!text.trim() || attachments.length > 0;
  const canSend = hasContent && ferry.available;
  const noCredential = health && health.credential === "unavailable";
  const currentModel = (ferry.models || []).find(m =>
    m.provider === health?.provider && m.id === health?.model);
  const modelLabel = currentModel?.name
    || (ferry.activeLog?.model || health?.model || "").split("/").pop();
  const effort = health?.thinking && health.thinking !== "off" ? health.thinking : null;
  const needsSetup = noCredential || !(ferry.models || []).length;
  useEffect(() => { if (autoFocus) taRef.current?.focus(); }, [autoFocus, taRef]);
  useLayoutEffect(() => {
    const el = taRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(180, Math.max(44, el.scrollHeight))}px`;
  }, [text, taRef]);
  return (
    <div className="agent-composer-shell">
      {mention && (
        <MentionMenu query={mention.query} sessions={scanSessions} onPick={onPickMention} />)}
      {running && <div className="agent-composer-activity" role="status">
        <span className="agent-activity-dot" />
        {activity || t("askferry:composer.working")}
      </div>}
      <div className="chat-composer agent-composer">
        {!!attachments.length && (
          <div style={{ display: "flex", flexWrap: "wrap", gap: 6, paddingBottom: 9 }}>
            {attachments.map(item => (
              <span key={sessionAttachmentKey(item)} style={{ display: "inline-flex",
                alignItems: "center", gap: 5, maxWidth: 240, padding: "4px 7px",
                borderRadius: 7, background: "var(--inset)", color: "var(--tx2)",
                fontSize: 11.5 }}>
                <ToolIcon tool={item.tool} size={14} />
                <span style={{ overflow: "hidden", textOverflow: "ellipsis",
                  whiteSpace: "nowrap" }}>{item.title}</span>
                <button onClick={() => onRemoveAttachment(item)}
                  title={t("askferry:composer.removeAttachment")}
                  style={{ border: 0, background: "transparent", color: "var(--tx4)",
                    padding: 0, lineHeight: 1, fontSize: 15, cursor: "default" }}>×</button>
              </span>
            ))}
          </div>
        )}
        <textarea className="agent-composer-input" aria-label={t("askferry:composer.placeholder")} ref={taRef} value={text}
          rows={Math.min(8, Math.max(1, text.split("\n").length))}
          onChange={e => setTextValue(e.target.value)} onKeyDown={onKeyDown} onPaste={onPaste}
          placeholder={ferry.available ? t("askferry:composer.placeholder")
            : t("askferry:composer.desktopOnly")}
          disabled={!ferry.available}
          style={{ width: "100%", border: "none", outline: "none", resize: "none",
            background: "transparent", fontSize: 13.5, lineHeight: 1.55, color: "var(--tx1)",
            fontFamily: "inherit", padding: 0 }} />
        <div className="agent-composer-toolbar">
          <div style={{ position: "relative" }}>
            {modeOpen && (
              <ModeMenu anchorRef={modeAnchorRef} mode={mode} onClose={() => setModeOpen(false)}
                onPick={k => { ferry.setMode(k); setModeOpen(false); }} />)}
            <button ref={modeAnchorRef} className="chat-ghost-btn" aria-haspopup="menu" aria-expanded={modeOpen}
              title={t(mode === "auto" ? "askferry:mode.autoDesc" : "askferry:mode.manualDesc")}
              onClick={() => { setModelOpen(false); setModeOpen(v => !v); }}>
              <span style={{ display: "inline-flex",
                color: mode === "auto" ? "var(--warn)" : "var(--tx3b)" }}>
                {mode === "auto" ? <AutoModeIcon size={13} /> : <ManualModeIcon size={13} />}
              </span>
              {t(mode === "auto" ? "askferry:mode.auto" : "askferry:mode.manual")}
              <Caret size={8} open={false} />
            </button>
          </div>
          <div className="agent-model-control" style={{ position: "relative" }}>
            {modelOpen && !needsSetup && (
              <ModelMenu anchorRef={modelAnchorRef} health={health} onManage={() => onOpenConfig("models")}
                onClose={() => setModelOpen(false)} />)}
            <button ref={modelAnchorRef} className="chat-ghost-btn" aria-haspopup="menu" aria-expanded={modelOpen}
              onClick={() => { setModeOpen(false); needsSetup ? onOpenConfig() : setModelOpen(v => !v); }}>
              {needsSetup ? (
                <span style={{ width: 5, height: 5, borderRadius: "50%",
                  background: "var(--warn)", flex: "none" }} />
              ) : currentModel && <ProviderIcon provider={currentModel.provider} size={13} />}
              <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {needsSetup ? t("askferry:model.setup")
                  : (modelLabel || t("askferry:model.pick"))}</span>
              {effort && (
                <span className="agent-effort-badge" title={t("askferry:model.effort")}>
                  {t(`askferry:model.effort_${effort}`)}</span>)}
              <Caret size={8} open={false} />
            </button>
          </div>
          <span style={{ flex: 1 }} />
          {running && hasContent && (
            <button className="chat-ghost-btn" title={t("askferry:composer.steerHint")} onClick={onSteer}>
              {t("askferry:composer.steer")}</button>
          )}
          {running && (
            <button className="chat-round-btn agent-stop-btn" title={t("askferry:composer.stop")}
              onClick={ferry.abort}><StopFillIcon /></button>
          )}
          {(!running || hasContent) && (
            <button className={`chat-round-btn agent-send-btn${running ? " is-queued" : ""}`} disabled={!canSend}
              title={running ? t("askferry:composer.followUp") : t("askferry:composer.send")}
              onClick={onSend}><SendArrowIcon />{running && <span>{t("askferry:composer.followUp")}</span>}</button>
          )}
        </div>
      </div>
      <div className="agent-composer-hint">
        <span>{t(running ? "askferry:composer.queueHint" : "askferry:composer.mentionHint")}</span>
        <span className="agent-keyboard-hint">{t("askferry:composer.keyboardHint")}</span>
      </div>
    </div>
  );
}
