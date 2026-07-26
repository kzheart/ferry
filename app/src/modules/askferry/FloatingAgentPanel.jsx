// 会话库的浮动 Agent 面板:浏览历史会话时右下角悬浮球,展开后就当前会话就地提问。
// 面板对话是真实的 Ask Ferry 会话(复用 runtime 管线与时间线组件),并按
// 「被浏览会话 → Agent 会话」映射记忆在 localStorage,再次查看同一会话可续聊。
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useFerryRuntime } from "../../shared/capabilities/ferryRuntime.jsx";
import { sessionAttachment, sessionIdentity } from "../browser/public.js";
import { useAgentComposer } from "./useAgentComposer.js";
import { CloseIcon, RailGlyph, Spinner, ToolIcon } from "../../shared/ui/icons.jsx";
import { groupAgentTimeline, isAwaitingReply } from "./agentTimelineModel.js";
import { AgentChatItem, ThinkingIndicator } from "./AgentChatItem.jsx";
import { AgentToolTrace } from "./AgentToolTrace.jsx";
import { AgentComposer } from "./AgentComposer.jsx";

const MAP_KEY = "ferry-float-chat-map";
const loadMap = () => {
  try { return JSON.parse(localStorage.getItem(MAP_KEY)) || {}; }
  catch { return {}; }
};

export default function FloatingAgentPanel({ open, onToggle, session, scanSessions,
  onOpenConfig, onOpenFull, onNavigate }) {
  const { t } = useTranslation();
  const ferry = useFerryRuntime();
  const identity = sessionIdentity(session);
  const [map, setMap] = useState(loadMap);
  const saveMap = next => { setMap(next); localStorage.setItem(MAP_KEY, JSON.stringify(next)); };

  const [attachments, setAttachments] = useState([]);
  const [unread, setUnread] = useState(false);
  const sessionsRef = useRef(ferry.sessions); sessionsRef.current = ferry.sessions;

  const boundId = identity ? map[identity] : null;
  const boundSession = boundId
    ? ferry.sessions.find(item => item.session_id === boundId) : null;
  const running = boundSession?.status === "running";
  const log = open ? ferry.activeLog : null;
  const items = log?.items || [];
  const empty = items.length === 0;

  const composer = useAgentComposer({
    attachments,
    setAttachments,
    logItems: log?.items,
    // 首条消息才会创建会话:发送前记下当时是否已有映射,成功后据此落库
    onBeforeSend: () => identity && !map[identity],
    onSent: (sid, wasUnbound) => {
      if (wasUnbound) saveMap({ ...map, [identity]: sid });
    },
    stopEscapePropagation: true,
  });
  const { setText, setMention, send: sendText } = composer;

  // 绑定:打开面板或切换被浏览会话时,续上已映射的对话;否则开新对话并预填上下文。
  // 依赖里刻意不含 ferry.sessions:首条消息创建会话会改列表,不能触发重新绑定。
  useEffect(() => {
    if (!open || !identity) return;
    const mapped = map[identity];
    const list = sessionsRef.current;
    if (mapped && (!list.length || list.some(item => item.session_id === mapped))) {
      if (ferry.activeId !== mapped) ferry.openSession(mapped);
      return;
    }
    if (mapped) {
      // 映射指向的会话已被删除:清掉失效映射
      const { [identity]: _stale, ...rest } = map;
      saveMap(rest);
    }
    ferry.newChat();
    const attachment = sessionAttachment(session);
    setAttachments(attachment ? [attachment] : []);
    setText(""); setMention(null);
  }, [open, identity, map]); // eslint-disable-line react-hooks/exhaustive-deps

  // 收起时后台运行结束 → 悬浮球未读标记
  const prevRunning = useRef(false);
  useEffect(() => {
    if (!open && prevRunning.current && !running) setUnread(true);
    prevRunning.current = running;
  }, [running, open]);
  useEffect(() => { if (open) setUnread(false); }, [open]);

  const chatRunning = log?.status === "running";
  const composerProps = { ...composer.composerProps, scanSessions,
    running: chatRunning, mode: ferry.mode, onOpenConfig, health: ferry.health };

  const suggestions = ["suggestSummary", "suggestFiles", "suggestIssues"];

  if (!identity) return null;
  return (
    <div style={{ position: "absolute", right: 20, bottom: 20, zIndex: 50,
      display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 10 }}>
      {open && (
        <div style={{ width: 400, height: "min(540px, calc(100vh - 140px))",
          display: "flex", flexDirection: "column", background: "var(--bg)",
          borderRadius: 14, boxShadow: "var(--shadow-menu)",
          border: "1px solid var(--line)", overflow: "hidden", position: "relative",
          animation: "fpop .16s ease" }}>
          {/* 头部:对话标题 + 上下文说明 + 打开完整视图 / 收起 */}
          <div style={{ flex: "none", display: "flex", alignItems: "center", gap: 8,
            padding: "10px 10px 8px 14px", borderBottom: "1px solid var(--line)" }}>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)",
                overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {boundSession?.title || t("askferry:float.title")}
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 4, marginTop: 1,
                fontSize: 10.5, color: "var(--tx5)", overflow: "hidden" }}>
                <ToolIcon tool={session.tool} size={12} />
                <span style={{ overflow: "hidden", textOverflow: "ellipsis",
                  whiteSpace: "nowrap" }}>
                  {t("askferry:float.aboutSession", { title: session.title || session.id })}
                </span>
              </div>
            </div>
            <button className="hov-item" onClick={onOpenFull}
              style={{ border: 0, background: "transparent", color: "var(--tx3b)",
                fontSize: 11, padding: "4px 7px", borderRadius: 6, cursor: "default",
                flex: "none" }}>
              {t("askferry:float.openFull")}
            </button>
            <button className="hov-item" onClick={onToggle} title={t("askferry:float.close")}
              style={{ border: 0, background: "transparent", color: "var(--tx4)",
                width: 24, height: 24, borderRadius: 6, display: "inline-flex",
                alignItems: "center", justifyContent: "center", cursor: "default",
                flex: "none" }}>
              <CloseIcon size={11} />
            </button>
          </div>

          {empty ? (
            /* 空态:建议问题 chips,点击直接发送 */
            <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column",
              justifyContent: "center", alignItems: "center", gap: 8, padding: "0 20px" }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: "var(--tx2)",
                marginBottom: 6 }}>{t("askferry:empty.title")}</div>
              {suggestions.map(key => (
                <button key={key} className="chat-chip"
                  onClick={() => sendText(t(`askferry:float.${key}`))}
                  style={{ padding: "6px 12px", borderRadius: 10, fontSize: 12,
                    color: "var(--tx2)" }}>
                  {t(`askferry:float.${key}`)}
                </button>
              ))}
            </div>
          ) : (
            <div ref={composer.scrollRef} onScroll={composer.onScroll} className="fscroll"
              style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "12px 14px 16px" }}>
              <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
                {groupAgentTimeline(items).map((group, index) => (
                  group.kind === "trace"
                    ? <AgentToolTrace key={`trace-${index}`} rows={group.rows}
                        onNavigate={onNavigate} />
                    : <AgentChatItem key={`item-${index}`} item={group}
                        sessionId={ferry.activeId} onNavigate={onNavigate} />))}
                {isAwaitingReply(log?.status, items) && <ThinkingIndicator />}
              </div>
            </div>
          )}

          <div style={{ flex: "none", padding: "0 10px 10px" }}>
            <AgentComposer {...composerProps} autoFocus />
          </div>

          {ferry.lastError && (
            <div onClick={ferry.clearError}
              style={{ position: "absolute", left: "50%", transform: "translateX(-50%)",
                bottom: 88, zIndex: 5, maxWidth: 340, padding: "7px 12px", borderRadius: 9,
                background: "var(--tooltip)", color: "#fff", fontSize: 11.5, lineHeight: 1.5,
                boxShadow: "var(--shadow-menu)", animation: "fpop .16s ease" }}>
              {String(ferry.lastError.message || ferry.lastError)}
            </div>
          )}
        </div>
      )}

      {/* 悬浮球:收起时显示运行中 / 未读状态 */}
      <button onClick={onToggle}
        title={open ? t("askferry:float.close") : t("askferry:float.open")}
        style={{ width: 46, height: 46, borderRadius: "50%", border: "none",
          background: "var(--accent)", color: "var(--accent-fg)", cursor: "default",
          display: "inline-flex", alignItems: "center", justifyContent: "center",
          boxShadow: "var(--shadow-menu)", position: "relative", flex: "none" }}>
        {open ? <CloseIcon size={13} /> : <RailGlyph name="askferry" color="var(--accent-fg)" size={19} />}
        {!open && running && (
          <span style={{ position: "absolute", top: -1, right: -1, width: 15, height: 15,
            borderRadius: "50%", background: "var(--bg)", display: "inline-flex",
            alignItems: "center", justifyContent: "center" }}>
            <Spinner size={10} />
          </span>
        )}
        {!open && !running && unread && (
          <span style={{ position: "absolute", top: 1, right: 1, width: 9, height: 9,
            borderRadius: "50%", background: "var(--warn)",
            border: "2px solid var(--bg)" }} />
        )}
      </button>
    </div>
  );
}
