// Ask Ferry 主聊天视图 —— 对齐 ChatGPT/Claude/Cursor 桌面端的对话形态:
// 头部只留标题;模式与模型选择器收进输入胶囊底部工具条(Cursor 式下拉);
// 未配置凭据时聊天框照常显示,模型按钮变成「配置模型」直达设置;空对话时输入框垂直居中。
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useFerryRuntime } from "../../shared/capabilities/ferryRuntime.jsx";
import { groupAgentTimeline, isAwaitingReply } from "./agentTimelineModel.js";
import { useAgentComposer } from "./useAgentComposer.js";
import { Caret, RoleAvatar } from "../../shared/ui/icons.jsx";
import { AgentChatItem, ThinkingIndicator } from "./AgentChatItem.jsx";
import { RoleMenu } from "./AgentMenus.jsx";
import { AgentComposer } from "./AgentComposer.jsx";
import { AgentToolTrace } from "./AgentToolTrace.jsx";

// ----- 主视图 -----
export default function AskFerry({ scanSessions, onOpenConfig,
  attachments, onAttachmentsChange, onNavigate }) {
  const { t } = useTranslation();
  const ferry = useFerryRuntime();
  const { activeId, activeLog, mode, health } = ferry;
  const activeSession = activeId
    ? ferry.sessions.find(session => session.session_id === activeId)
    : null;
  const [roleOpen, setRoleOpen] = useState(false);
  const selectedRole = (ferry.roles || [])
    .find(role => role.id === ferry.selectedRoleId);
  const running = activeLog?.status === "running";
  const items = activeLog?.items || [];
  const empty = items.length === 0;
  // 流式期间每个 token 都会换一次 items:不 memo 的话每帧都要把整条时间线
  // 重新分组一遍。
  const groups = useMemo(() => groupAgentTimeline(items), [items]);

  const composer = useAgentComposer({
    attachments,
    setAttachments: onAttachmentsChange,
    logItems: activeLog?.items,
  });

  // 错误 toast:6 秒自动消失
  useEffect(() => {
    if (!ferry.lastError) return;
    const id = setTimeout(ferry.clearError, 6000);
    return () => clearTimeout(id);
  }, [ferry.lastError, ferry.clearError]);

  const composerProps = { ...composer.composerProps, scanSessions, running, mode,
    onOpenConfig, health };

  return (
    <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column",
      position: "relative" }}>
      {/* 头部:居中标题;新会话时左上角放角色胶囊(角色在会话创建时快照,只在新会话可选) */}
      <div style={{ flex: "none", padding: "0 16px 8px", textAlign: "center",
        position: "relative" }}>
        {!activeId && selectedRole && (
          <div style={{ position: "absolute", left: 20, top: -4, zIndex: 10 }}>
            {roleOpen && (
              <RoleMenu onClose={() => setRoleOpen(false)}
                onManage={() => onOpenConfig("roles")}
                menuStyle={{ top: "100%", bottom: "auto",
                  marginTop: 8, marginBottom: 0 }} />)}
            <button className="chat-chip" title={selectedRole.description || undefined}
              onClick={() => setRoleOpen(value => !value)}
              style={{ display: "inline-flex", alignItems: "center", gap: 8,
                padding: "6px 13px 6px 8px", borderRadius: 11 }}>
              <RoleAvatar icon={selectedRole.icon} color={selectedRole.color} size={26} />
              <span style={{ textAlign: "left" }}>
                <span style={{ display: "block", fontSize: 10, lineHeight: 1.2,
                  color: "var(--tx5)" }}>
                  {t("askferry:role.label")}</span>
                <span style={{ display: "block", fontSize: 13, lineHeight: 1.25,
                  fontWeight: 600, color: "var(--tx1)" }}>
                  {selectedRole.name}</span>
              </span>
              <Caret size={9} open={roleOpen} />
            </button>
          </div>
        )}
        <span style={{ fontSize: 13, fontWeight: 600, color: "var(--tx1)",
          display: "inline-block", maxWidth: "70%", overflow: "hidden",
          textOverflow: "ellipsis", whiteSpace: "nowrap", verticalAlign: "bottom" }}>
          {activeId ? (activeSession?.title || t("askferry:chat.untitled")) : t("askferry:chat.newChat")}
        </span>
      </div>

      {empty ? (
        /* 空态:问候语 + 居中输入框 + 角色 chips(角色在会话创建时快照,只在新会话可选);
           未配置模型也照常显示 */
        <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column",
          justifyContent: "center", padding: "0 24px 60px" }}>
          <div style={{ width: "100%", maxWidth: 640, margin: "0 auto" }}>
            <div style={{ fontSize: 22, fontWeight: 600, color: "var(--tx1)",
              textAlign: "center", letterSpacing: "-.01em", marginBottom: 22 }}>
              {t("askferry:empty.title")}</div>
            <AgentComposer {...composerProps} autoFocus />
          </div>
        </div>
      ) : (
        <>
          {/* 消息流 */}
          <div ref={composer.scrollRef} onScroll={composer.onScroll} className="fscroll"
            style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "18px 24px 24px" }}>
            <div style={{ maxWidth: 680, margin: "0 auto", display: "flex",
              flexDirection: "column", gap: 14 }}>
              {groups.map((g, i) => (
                g.kind === "trace"
                  ? <AgentToolTrace key={`trace-${i}`} rows={g.rows} onNavigate={onNavigate} />
                  : <AgentChatItem key={g.callId || g.requestId || `item-${i}`}
                      item={g} sessionId={activeId}
                      onNavigate={onNavigate} />))}
              {isAwaitingReply(activeLog?.status, items) && <ThinkingIndicator />}
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
