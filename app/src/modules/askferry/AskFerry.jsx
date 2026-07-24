// Ask Ferry 主聊天视图 —— 对齐 ChatGPT/Claude/Cursor 桌面端的对话形态:
// 头部只留标题;模式与模型选择器收进输入胶囊底部工具条(Cursor 式下拉);
// 未配置凭据时聊天框照常显示,模型按钮变成「配置模型」直达设置;空对话时输入框垂直居中。
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { readClipboardText } from "../../platform/desktop/client.js";
import { groupAgentTimeline } from "./agentTimelineModel.js";
import { addSessionAttachment, buildSessionPrompt, parseSessionAttachments,
  sessionAttachmentKey, sessionDisplayText }
  from "../browser/sessionAttachment.js";
import { AgentChatItem } from "./AgentChatItem.jsx";
import { AgentComposer } from "./AgentComposer.jsx";
import { AgentToolTrace } from "./AgentToolTrace.jsx";

// ----- 主视图 -----
export default function AskFerry({ ferry, scanSessions, onOpenConfig,
  attachments, onAttachmentsChange, onNavigate }) {
  const { t } = useTranslation();
  const { activeId, activeLog, mode, health } = ferry;
  const activeSession = activeId
    ? ferry.sessions.find(session => session.session_id === activeId)
    : null;
  const [text, setText] = useState("");
  const [mention, setMention] = useState(null); // {query, start}
  const taRef = useRef(null);
  const scrollRef = useRef(null);
  const running = activeLog?.status === "running";
  const items = activeLog?.items || [];
  const empty = items.length === 0;

  // 新消息时贴底滚动(用户上翻后不打扰)
  const stickRef = useRef(true);
  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickRef.current) el.scrollTop = el.scrollHeight;
  }, [activeLog?.items]);

  // 错误 toast:6 秒自动消失
  useEffect(() => {
    if (!ferry.lastError) return;
    const id = setTimeout(ferry.clearError, 6000);
    return () => clearTimeout(id);
  }, [ferry.lastError, ferry.clearError]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (el) stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
  };

  const updateText = value => {
    setText(value);
    const el = taRef.current;
    const caret = el ? el.selectionStart : value.length;
    const before = value.slice(0, caret);
    const m = before.match(/@([^\s@]*)$/);
    setMention(m ? { query: m[1], start: caret - m[0].length } : null);
  };

  const pickMention = s => {
    const el = taRef.current;
    const caret = el ? el.selectionStart : text.length;
    setText(text.slice(0, mention.start) + text.slice(caret));
    onAttachmentsChange(list => addSessionAttachment(list, s));
    setMention(null);
    el?.focus();
  };

  const removeAttachment = target => onAttachmentsChange(list =>
    list.filter(item => sessionAttachmentKey(item) !== sessionAttachmentKey(target)));

  const applyClipboardText = pastedText => {
    const pasted = parseSessionAttachments(pastedText);
    if (pasted.length) {
      onAttachmentsChange(list => pasted.reduce(addSessionAttachment, list));
      return;
    }
    if (!pastedText) return;
    const el = taRef.current;
    const start = el?.selectionStart ?? text.length;
    const end = el?.selectionEnd ?? start;
    const next = text.slice(0, start) + pastedText + text.slice(end);
    updateText(next);
    requestAnimationFrame(() => {
      const caret = start + pastedText.length;
      taRef.current?.setSelectionRange(caret, caret);
    });
  };

  const onPaste = event => {
    const pastedText = event.clipboardData?.getData("text/plain") || "";
    if (!pastedText && window.__TAURI_INTERNALS__) {
      event.preventDefault();
      readClipboardText().then(applyClipboardText).catch(() => {});
      return;
    }
    const pasted = parseSessionAttachments(pastedText);
    if (!pasted.length) return;
    event.preventDefault();
    onAttachmentsChange(list => pasted.reduce(addSessionAttachment, list));
  };

  const payload = value => ({
    prompt: buildSessionPrompt(value, attachments),
    display: sessionDisplayText(value, attachments),
  });

  const doSend = async () => {
    const value = text.trim();
    if (!value && !attachments.length) return;
    const currentAttachments = attachments;
    const message = payload(value);
    setText(""); onAttachmentsChange([]); setMention(null); stickRef.current = true;
    try { await ferry.send(message.prompt, message.display); }
    catch (error) {
      setText(value); onAttachmentsChange(currentAttachments); ferry.reportError(error);
    }
  };

  const doSteer = async () => {
    const value = text.trim();
    if (!value && !attachments.length) return;
    const currentAttachments = attachments;
    const message = payload(value);
    setText(""); onAttachmentsChange([]); setMention(null);
    try { await ferry.steer(message.prompt, message.display); }
    catch (error) {
      setText(value); onAttachmentsChange(currentAttachments); ferry.reportError(error);
    }
  };

  const onKeyDown = e => {
    if (window.__TAURI_INTERNALS__ && (e.metaKey || e.ctrlKey)
        && e.key.toLowerCase() === "v") {
      e.preventDefault();
      readClipboardText().then(applyClipboardText).catch(() => {});
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && !mention) {
      e.preventDefault();
      doSend();
    }
    if (e.key === "Escape" && mention) setMention(null);
  };

  const composerProps = { ferry, text, setTextValue: updateText, taRef, mention, scanSessions,
    onPickMention: pickMention, onKeyDown, onPaste, onSend: doSend, onSteer: doSteer,
    running, mode, onOpenConfig, health, attachments, onRemoveAttachment: removeAttachment };

  return (
    <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column",
      position: "relative" }}>
      {/* 头部:只有标题 */}
      <div style={{ flex: "none", padding: "0 16px 8px", textAlign: "center" }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: "var(--tx1)",
          display: "inline-block", maxWidth: "70%", overflow: "hidden",
          textOverflow: "ellipsis", whiteSpace: "nowrap", verticalAlign: "bottom" }}>
          {activeId ? (activeSession?.title || t("askferry:chat.untitled")) : t("askferry:chat.newChat")}
        </span>
      </div>

      {empty ? (
        /* 空态:问候语 + 居中输入框 + 建议 chips;未配置模型也照常显示 */
        <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column",
          justifyContent: "center", padding: "0 24px 60px" }}>
          <div style={{ width: "100%", maxWidth: 640, margin: "0 auto" }}>
            <div style={{ fontSize: 22, fontWeight: 600, color: "var(--tx1)",
              textAlign: "center", letterSpacing: "-.01em", marginBottom: 22 }}>
              {t("askferry:empty.title")}</div>
            <AgentComposer {...composerProps} autoFocus />
            <div style={{ display: "flex", flexWrap: "wrap", gap: 8,
              justifyContent: "center", marginTop: 18 }}>
              {[t("askferry:empty.ex1"), t("askferry:empty.ex2"),
                t("askferry:empty.ex3"), t("askferry:empty.ex4")].map((ex, i) => (
                <button key={i} className="chat-chip"
                  onClick={() => { updateText(ex); taRef.current?.focus(); }}>{ex}</button>
              ))}
            </div>
          </div>
        </div>
      ) : (
        <>
          {/* 消息流 */}
          <div ref={scrollRef} onScroll={onScroll} className="fscroll"
            style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "18px 24px 24px" }}>
            <div style={{ maxWidth: 680, margin: "0 auto", display: "flex",
              flexDirection: "column", gap: 14 }}>
              {groupAgentTimeline(items).map((g, i) => (
                g.kind === "trace"
                  ? <AgentToolTrace key={`trace-${i}`} rows={g.rows} onNavigate={onNavigate} />
                  : <AgentChatItem key={`item-${i}`} item={g} sessionId={activeId}
                      ferry={ferry} onNavigate={onNavigate} />))}
            </div>
          </div>

          {/* 底部输入区 */}
          <div style={{ flex: "none", padding: "0 24px 16px" }}>
            <div style={{ maxWidth: 680, margin: "0 auto" }}>
              <AgentComposer {...composerProps} />
            </div>
          </div>
        </>
      )}

      {/* 错误 toast:底部居中浮层,自动消失 */}
      {ferry.lastError && (
        <div onClick={ferry.clearError}
          style={{ position: "absolute", left: "50%", transform: "translateX(-50%)",
            bottom: 96, zIndex: 40, maxWidth: 480, padding: "8px 14px", borderRadius: 10,
            background: "var(--tooltip)", color: "#fff", fontSize: 12, lineHeight: 1.5,
            boxShadow: "var(--shadow-menu)", animation: "fpop .16s ease", cursor: "default" }}>
          {String(ferry.lastError.message || ferry.lastError)}
        </div>
      )}
    </div>
  );
}
