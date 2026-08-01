import { memo, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { writeClipboardText } from "../../platform/desktop/client.js";
import { useFerryRuntime } from "../../shared/capabilities/ferryRuntime.jsx";
import Markdown from "../../shared/ui/Markdown.jsx";
import { CheckIcon, CloseIcon, CopyIcon, PencilIcon, SendArrowIcon, Spinner }
  from "../../shared/ui/icons.jsx";
import { AgentToolRow } from "./AgentToolTrace.jsx";
import { ApprovalCard } from "./AgentApprovalCard.jsx";
import { AgentChoiceCard } from "./AgentChoiceCard.jsx";

function IconBtn({ title, onClick, children }) {
  return (
    <button className="ficon-btn" title={title} onClick={onClick}>
      {children}
    </button>
  );
}

// 复制按钮:点击后短暂变对勾,对齐 SessionRound 的既有交互
function CopyBtn({ text }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const timerRef = useRef(null);
  useEffect(() => () => clearTimeout(timerRef.current), []);
  return (
    <IconBtn title={t(copied ? "askferry:chat.copied" : "askferry:chat.copy")}
      onClick={async () => {
        await writeClipboardText(text);
        setCopied(true);
        clearTimeout(timerRef.current);
        timerRef.current = setTimeout(() => setCopied(false), 1400);
      }}>
      {copied ? <CheckIcon /> : <CopyIcon />}
    </IconBtn>
  );
}

const fitTextArea = el => {
  if (!el) return;
  el.style.height = "auto";
  el.style.height = `${el.scrollHeight}px`;
};

function UserMessage({ item, sessionId }) {
  const { t } = useTranslation();
  const ferry = useFerryRuntime();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const areaRef = useRef(null);
  const running = ferry.activeLog?.status === "running";
  // steer/follow_up 插在一轮中间,带图消息编辑后无法还原图片:都只给复制
  const canEdit = !!sessionId && item.seq != null && !item.sub
    && !item.imageCount && !running;

  // 打开编辑时聚焦并把光标放到文本末尾(autoFocus 默认落在开头)
  useEffect(() => {
    if (!editing) return;
    const el = areaRef.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, [editing]);

  const submit = () => {
    const text = draft.trim();
    if (!text) return;
    setEditing(false);
    ferry.editResend(sessionId, item.seq, text);
  };

  if (editing) {
    return (
      <div className="chat-edit">
        <textarea ref={el => { areaRef.current = el; fitTextArea(el); }}
          className="selectable" value={draft}
          onChange={e => { setDraft(e.target.value); fitTextArea(areaRef.current); }}
          onKeyDown={e => {
            if (e.key === "Escape") setEditing(false);
            else if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              submit();
            }
          }} />
        <div style={{ display: "flex", gap: 6, justifyContent: "flex-end",
          padding: "0 8px 8px" }}>
          <IconBtn title={t("askferry:chat.editCancel")}
            onClick={() => setEditing(false)}>
            <CloseIcon />
          </IconBtn>
          <button className="chat-round-btn" title={t("askferry:chat.editSend")}
            onClick={submit} disabled={!draft.trim()}
            style={{ width: 26, height: 26 }}>
            <SendArrowIcon size={12} />
          </button>
        </div>
      </div>
    );
  }
  return (
    <div className="chat-msg"
      style={{ display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 3 }}>
      <div className="chat-user selectable">{item.text}</div>
      <div className="fhact" style={{ display: "flex", gap: 2 }}>
        <CopyBtn text={item.text} />
        {canEdit && (
          <IconBtn title={t("askferry:chat.edit")}
            onClick={() => { setDraft(item.text); setEditing(true); }}>
            <PencilIcon />
          </IconBtn>
        )}
      </div>
    </div>
  );
}

function AgentChatItemView({ item, sessionId, onNavigate }) {
  const { t } = useTranslation();
  const ferry = useFerryRuntime();
  if (item.kind === "user") {
    return <UserMessage item={item} sessionId={sessionId} />;
  }
  if (item.kind === "assistant") {
    return (
      <div className="selectable chat-msg">
        <Markdown text={item.text} />
        {item.streaming
          ? <div style={{ marginTop: 6 }}><Spinner size={12} /></div>
          : (
            <div className="fhact" style={{ display: "flex", marginTop: 2, marginLeft: -7 }}>
              <CopyBtn text={item.text} />
            </div>
          )}
      </div>
    );
  }
  if (item.kind === "tool") {
    return <AgentToolRow item={item} onNavigate={onNavigate} />;
  }
  if (item.kind === "approval") {
    return (
      <ApprovalCard item={item}
        onApprove={() => ferry.approve(sessionId, item)}
        onDismiss={() => ferry.dismiss(sessionId, item)}
      onNavigate={onNavigate} />
    );
  }
  if (item.kind === "choice") {
    return (
      <AgentChoiceCard item={item}
        onRespond={answer => ferry.respondChoice(sessionId, item.requestId, answer)} />
    );
  }
  if (item.kind === "status") {
    const status = {
      "run.failed": [
        "var(--err-text)",
        t("askferry:chat.runFailed", { message: item.message || "" }),
      ],
      "run.cancelled": ["var(--tx5)", t("askferry:chat.runCancelled")],
      "run.interrupted": ["var(--warn-text)", t("askferry:chat.runInterrupted")],
    };
    const [color, label] = status[item.type] || ["var(--tx5)", item.type];
    return (
      <div style={{ fontSize: 11.5, color, textAlign: "center", padding: "2px 0" }}>
        {label}
      </div>
    );
  }
  return null;
}

// 流式期间每个 token 都会重渲染整条消息流,但只有最后一条 assistant 在变。
export const AgentChatItem = memo(AgentChatItemView);

// 等待回复占位:呼吸圆点 + 文案,配合 isAwaitingReply 使用
export function ThinkingIndicator() {
  const { t } = useTranslation();
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8,
      color: "var(--tx5)", fontSize: 12 }}>
      <span style={{ width: 8, height: 8, borderRadius: "50%",
        background: "var(--tx4)", animation: "fbreath 1.2s ease-in-out infinite" }} />
      {t("askferry:chat.thinking")}
    </div>
  );
}
