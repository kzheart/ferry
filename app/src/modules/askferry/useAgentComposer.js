// Agent 输入框的交互内核:@mention、附件粘贴、发送/插话、贴底滚动。
// 主视图与浮动面板共用;差异只有附件状态的宿主(受控 / 内部)与发送后的回调。
import { useEffect, useRef, useState } from "react";
import { readClipboardText } from "../../platform/desktop/client.js";
import { useFerryRuntime } from "../../shared/capabilities/ferryRuntime.jsx";
import { addSessionAttachment, buildSessionPrompt, parseSessionAttachments,
  sessionAttachmentKey, sessionDisplayText } from "../browser/public.js";

export function useAgentComposer({
  attachments,
  setAttachments,
  logItems,
  // 发送前取一份快照,发送成功后连同 session_id 交回给调用方(浮动面板据此建立映射)
  onBeforeSend,
  onSent,
  // 浮动面板的 Esc 要就地消化,否则会一路冒泡把面板关掉
  stopEscapePropagation = false,
}) {
  const ferry = useFerryRuntime();
  const [text, setText] = useState("");
  const [mention, setMention] = useState(null); // {query, start}
  const taRef = useRef(null);
  const scrollRef = useRef(null);
  const stickRef = useRef(true);

  // 新消息时贴底滚动(用户上翻后不打扰)
  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickRef.current) el.scrollTop = el.scrollHeight;
  }, [logItems]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (el) stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
  };

  const updateText = value => {
    setText(value);
    const el = taRef.current;
    const caret = el ? el.selectionStart : value.length;
    const m = value.slice(0, caret).match(/@([^\s@]*)$/);
    setMention(m ? { query: m[1], start: caret - m[0].length } : null);
  };

  const pickMention = target => {
    const el = taRef.current;
    const caret = el ? el.selectionStart : text.length;
    setText(text.slice(0, mention.start) + text.slice(caret));
    setAttachments(list => addSessionAttachment(list, target));
    setMention(null);
    el?.focus();
  };

  const removeAttachment = target => setAttachments(list =>
    list.filter(item => sessionAttachmentKey(item) !== sessionAttachmentKey(target)));

  const applyClipboardText = pastedText => {
    const pasted = parseSessionAttachments(pastedText);
    if (pasted.length) {
      setAttachments(list => pasted.reduce(addSessionAttachment, list));
      return;
    }
    if (!pastedText) return;
    const el = taRef.current;
    const start = el?.selectionStart ?? text.length;
    const end = el?.selectionEnd ?? start;
    updateText(text.slice(0, start) + pastedText + text.slice(end));
    // 受控 textarea 重渲染后光标会跳到末尾,粘到中间时要把它挪回插入点之后
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
    setAttachments(list => pasted.reduce(addSessionAttachment, list));
  };

  // 发送失败要把输入与附件原样退回,否则用户白打一遍
  const runWith = async (value, transmit, stick) => {
    if (!value && !attachments.length) return undefined;
    const currentAttachments = attachments;
    const prompt = buildSessionPrompt(value, currentAttachments);
    const display = sessionDisplayText(value, currentAttachments);
    setText(""); setAttachments([]); setMention(null);
    if (stick) stickRef.current = true;
    try { return await transmit(prompt, display); }
    catch (error) {
      setText(value); setAttachments(currentAttachments); ferry.reportError(error);
      return undefined;
    }
  };

  const send = async value => {
    const token = onBeforeSend?.();
    const sid = await runWith(value, (prompt, display) => ferry.send(prompt, display), true);
    if (sid) onSent?.(sid, token);
    return sid;
  };

  const doSend = () => send(text.trim());
  const doSteer = () => runWith(text.trim(), (prompt, display) => ferry.steer(prompt, display), false);

  const onKeyDown = event => {
    if (window.__TAURI_INTERNALS__ && (event.metaKey || event.ctrlKey)
        && event.key.toLowerCase() === "v") {
      event.preventDefault();
      readClipboardText().then(applyClipboardText).catch(() => {});
      return;
    }
    if (event.key === "Enter" && !event.shiftKey && !mention) {
      event.preventDefault();
      doSend();
    }
    if (event.key === "Escape" && mention) {
      if (stopEscapePropagation) event.stopPropagation();
      setMention(null);
    }
  };

  return {
    text, setText, mention, setMention, taRef, scrollRef, onScroll, send,
    composerProps: {
      text, setTextValue: updateText, taRef, mention,
      onPickMention: pickMention, onKeyDown, onPaste,
      onSend: doSend, onSteer: doSteer,
      attachments, onRemoveAttachment: removeAttachment,
    },
  };
}
