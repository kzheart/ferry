// Ask Ferry 主聊天视图 —— 对齐 ChatGPT/Claude/Cursor 桌面端的对话形态:
// 头部只留标题;模式与模型选择器收进输入胶囊底部工具条(Cursor 式下拉);
// 未配置凭据时聊天框照常显示,模型按钮变成「配置模型」直达设置;空对话时输入框垂直居中。
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import Markdown from "../../components/ui/Markdown.jsx";
import { AutoModeIcon, Caret, CheckIcon, ManualModeIcon, ProviderIcon, SendArrowIcon,
  Spinner, StopFillIcon, ToolIcon } from "../../components/ui/icons.jsx";
import { readClipboardText } from "../../api/transport/rpc.js";
import { TOOL_LEVEL } from "../../domain/agent/agentChatModel.js";
import { addSessionAttachment, buildSessionPrompt, parseSessionAttachments,
  sessionAttachmentKey, sessionDisplayText }
  from "../../domain/sessions/sessionAttachment.js";
import { entitiesFromToolResult } from "../../domain/entities/ferryEntities.js";
import EntityCards from "./EntityCards.jsx";

const fmtDur = (a, b) => {
  if (!a || !b) return "";
  const s = (new Date(b) - new Date(a)) / 1000;
  return s >= 0 ? `${s < 10 ? s.toFixed(1) : Math.round(s)}s` : "";
};

// 工具结果多为 JSON 字符串,能解析就缩进展示,解析不了按原文
const prettyJson = text => {
  if (typeof text !== "string") return "";
  try { return JSON.stringify(JSON.parse(text), null, 2); }
  catch { return text; }
};

const preStyle = { margin: 0, padding: "8px 12px", fontSize: 11, lineHeight: 1.55,
  color: "var(--tx3)", background: "var(--inset)", border: "1px solid var(--line5)",
  borderRadius: 9, overflow: "auto", maxHeight: 260, whiteSpace: "pre-wrap",
  wordBreak: "break-word" };
const secLabel = { fontSize: 10.5, color: "var(--tx5)", margin: "0 0 3px 2px" };

// ----- 工具调用:一行安静的灰字,点击展开参数与结果 -----
const ToolRow = memo(function ToolRow({ item, onNavigate }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const level = TOOL_LEVEL[item.name] || "read";
  const resultText = item.result?.text ? prettyJson(item.result.text) : "";
  return (
    <div>
      <div className="chat-tool" onClick={() => setOpen(v => !v)}>
        {item.status === "running" ? <Spinner size={11} />
          : <Caret open={open} size={8} />}
        <span className="mono" style={{ fontSize: 11.5, overflow: "hidden",
          textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{item.name}</span>
        {level === "mutate" && (
          <span style={{ width: 5, height: 5, borderRadius: "50%", flex: "none",
            background: "var(--warn)" }} />)}
        {item.status === "error" && (
          <span style={{ fontSize: 11, color: "var(--err-text)", flex: "none" }}>
            {t("askferry:tool.failed")}</span>)}
        {item.status !== "running" && (
          <span style={{ fontSize: 10.5, color: "var(--tx5)", flex: "none" }}>
            {fmtDur(item.startedAt, item.endedAt)}</span>)}
      </div>
      {open && (
        <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 5 }}>
          <div>
            <div style={secLabel}>{t("askferry:tool.args")}</div>
            <pre className="mono selectable" style={preStyle}>
              {JSON.stringify(item.args ?? {}, null, 2)}
            </pre>
          </div>
          {item.status !== "running" && (
            <div>
              <div style={secLabel}>
                {item.status === "error" ? t("askferry:tool.errorResult") : t("askferry:tool.result")}
                {item.result?.truncated && ` · ${t("askferry:tool.truncated")}`}
              </div>
              <pre className="mono selectable" style={{ ...preStyle,
                color: item.status === "error" ? "var(--err-text)" : preStyle.color }}>
                {resultText || t("askferry:tool.noResult")}
              </pre>
            </div>
          )}
        </div>
      )}
      <EntityCards entities={item.entities} onNavigate={onNavigate} />
    </div>
  );
});

// ----- 审批卡倒计时 -----
function Countdown({ until }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  const left = Math.max(0, Math.floor(((until || 0) - now) / 1000));
  return <span className="mono">{Math.floor(left / 60)}:{String(left % 60).padStart(2, "0")}</span>;
}

// ----- 审批卡:白底细线卡片,状态只通过左侧色点表达 -----
const KIND_KEYS = { migration: "kindMigration", edit: "kindEdit", metadata: "kindMetadata" };
function ApprovalCard({ item, onApprove, onDismiss, onNavigate }) {
  const { t } = useTranslation();
  const op = item.operation || {};
  const applied = item.status === "applied";
  const failed = item.status === "failed";
  const expired = item.status === "pending" && op.expires_at && op.expires_at < Date.now();
  const dot = applied ? "var(--ok)" : failed ? "var(--err)" : "var(--warn)";
  const title = applied
    ? (item.auto ? t("askferry:approval.autoApplied") : t("askferry:approval.applied"))
    : failed ? t("askferry:approval.failed")
    : item.status === "applying" ? t("askferry:approval.applying")
    : item.status === "dismissed" ? t("askferry:approval.dismissed")
    : t(`askferry:approval.${KIND_KEYS[op.kind] || "kindGeneric"}`);
  const entities = op.kind === "migration" || op.kind === "edit"
    ? entitiesFromToolResult(
        op.kind === "migration" ? "migrate" : "session_edit",
        { details: { ...op, result: item.result } },
      )
    : [];
  return (
    <div className="fcard" style={{ padding: "12px 14px", display: "flex",
      flexDirection: "column", gap: 8, maxWidth: 560 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={{ width: 7, height: 7, borderRadius: "50%", background: dot, flex: "none" }} />
        <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)" }}>{title}</span>
        <span style={{ flex: 1 }} />
        {item.status === "pending" && op.expires_at && !expired && (
          <span style={{ fontSize: 11, color: "var(--tx5)" }}>
            {t("askferry:approval.expires")} <Countdown until={op.expires_at} /></span>)}
      </div>
      {op.summary && (
        <div className="selectable" style={{ fontSize: 12.5, color: "var(--tx2)", lineHeight: 1.55 }}>
          {op.summary}</div>)}
      <EntityCards entities={entities} onNavigate={onNavigate} />
      <div style={{ display: "flex", gap: 12, flexWrap: "wrap", fontSize: 11, color: "var(--tx4)" }}>
        {Array.isArray(op.affected_refs) && (
          <span>{t("askferry:approval.affected", { n: op.affected_refs.length })}</span>)}
        {op.risk && <span>{t("askferry:approval.risk", { risk: op.risk })}</span>}
        {expired && <span style={{ color: "var(--err-text)" }}>{t("askferry:approval.expired")}</span>}
      </div>
      {failed && item.error && (
        <div className="mono selectable" style={{ fontSize: 11, color: "var(--err-text)" }}>
          {item.error}</div>)}
      {applied && item.result?.saved_as && (
        <div className="mono selectable" style={{ fontSize: 11, color: "var(--tx4)" }}>
          {item.result.saved_as}</div>)}
      {item.status === "pending" && !expired && (
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end", marginTop: 2 }}>
          <button className="fbtn" onClick={onDismiss}>{t("askferry:approval.reject")}</button>
          <button className="fbtn fbtn-primary" onClick={onApprove}>
            {t("askferry:approval.approve")}</button>
        </div>
      )}
    </div>
  );
}

// ----- @ 提及菜单 -----
function MentionMenu({ query, sessions, onPick }) {
  const q = query.toLowerCase();
  const matched = useMemo(() => sessions
    .filter(s => !q || (s.title || "").toLowerCase().includes(q) || (s.id || "").toLowerCase().includes(q))
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
          <span style={{ fontSize: 10.5, color: "var(--tx5)" }}>{s.dir?.split("/").pop()}</span>
        </div>
      ))}
    </div>
  );
}

// ----- 消息项分发 -----
function ChatItem({ item, sessionId, ferry, onNavigate }) {
  const { t } = useTranslation();
  if (item.kind === "user") {
    return (
      <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 3 }}>
        {item.sub && (
          <span style={{ fontSize: 10.5, color: "var(--tx5)", paddingRight: 6 }}>
            {t(item.sub === "steer" ? "askferry:chat.steered" : "askferry:chat.followedUp")}</span>)}
        <div className="chat-user selectable">{item.text}</div>
      </div>
    );
  }
  if (item.kind === "assistant") {
    return (
      <div className="selectable">
        <Markdown text={item.text} />
        {item.streaming && <div style={{ marginTop: 6 }}><Spinner size={12} /></div>}
      </div>
    );
  }
  if (item.kind === "tool") return <ToolRow item={item} onNavigate={onNavigate} />;
  if (item.kind === "approval") {
    return <ApprovalCard item={item}
      onNavigate={onNavigate}
      onApprove={() => ferry.approve(sessionId, item)}
      onDismiss={() => ferry.dismiss(sessionId, item)} />;
  }
  if (item.kind === "status") {
    const map = { "run.failed": ["var(--err-text)", t("askferry:chat.runFailed", { message: item.message || "" })],
      "run.cancelled": ["var(--tx5)", t("askferry:chat.runCancelled")],
      "run.interrupted": ["var(--warn-text)", t("askferry:chat.runInterrupted")] };
    const [color, label] = map[item.type] || ["var(--tx5)", item.type];
    return <div style={{ fontSize: 11.5, color, textAlign: "center", padding: "2px 0" }}>{label}</div>;
  }
  return null;
}

// ----- 模式下拉(Cursor 式:选项带一行说明) -----
function ModeMenu({ mode, onPick, onClose }) {
  const { t } = useTranslation();
  const options = [
    ["manual", ManualModeIcon, t("askferry:mode.manual"), t("askferry:mode.manualDesc")],
    ["auto", AutoModeIcon, t("askferry:mode.auto"), t("askferry:mode.autoDesc")],
  ];
  return (
    <>
      <div onMouseDown={onClose} style={{ position: "fixed", inset: 0, zIndex: 29 }} />
      <div style={{ position: "absolute", left: 0, bottom: "100%", marginBottom: 8, width: 240,
        background: "var(--bg)", borderRadius: 11, boxShadow: "var(--shadow-menu)",
        padding: 4, zIndex: 30, animation: "fpop .14s ease" }}>
        {options.map(([k, Icon, name, desc]) => (
          <div key={k} className="hov-item"
            onMouseDown={e => { e.preventDefault(); onPick(k); }}
            style={{ padding: "7px 9px", borderRadius: 7, cursor: "default" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <span style={{ display: "inline-flex",
                color: k === "auto" ? "var(--warn)" : "var(--tx3b)" }}>
                <Icon />
              </span>
              <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)", flex: 1 }}>
                {name}</span>
              {mode === k && <CheckIcon size={12} />}
            </div>
            <div style={{ fontSize: 11, color: "var(--tx4)", lineHeight: 1.45, marginTop: 2 }}>
              {desc}</div>
          </div>
        ))}
      </div>
    </>
  );
}

function RoleMenu({ ferry, onClose, onManage }) {
  const { t } = useTranslation();
  return (
    <>
      <div onMouseDown={onClose} style={{ position: "fixed", inset: 0, zIndex: 29 }} />
      <div style={{ ...menuShell, width: 250 }}>
        {(ferry.roles || []).map(role => (
          <button key={role.id} type="button" className="hov-item"
            onMouseDown={event => {
              event.preventDefault();
              ferry.setSelectedRoleId(role.id);
              onClose();
            }}
            style={{ ...menuRow, width: "100%", border: "none",
              background: role.id === ferry.selectedRoleId ? "var(--acc-soft5)" : "transparent",
              fontFamily: "inherit", textAlign: "left" }}>
            <span style={{ flex: 1, minWidth: 0 }}>
              <span style={{ display: "block", fontSize: 12.5, fontWeight: 600,
                color: "var(--tx1)" }}>{role.name}</span>
              <span style={{ display: "block", fontSize: 10.5, color: "var(--tx5)",
                overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {role.description || role.tools?.join(" · ")}
              </span>
            </span>
            {role.id === ferry.selectedRoleId && <CheckIcon size={12} />}
          </button>
        ))}
        <div style={menuDivider} />
        <button type="button" className="hov-item"
          onMouseDown={event => { event.preventDefault(); onManage(); onClose(); }}
          style={{ ...menuRow, width: "100%", border: "none", background: "transparent",
            fontFamily: "inherit", fontSize: 12, color: "var(--tx2)" }}>
          {t("askferry:role.manage")}
        </button>
      </div>
    </>
  );
}

// ----- 模型下拉:一级选模型,二级选推理强度,底部通往设置里的 Provider 配置 -----
const EFFORT_LEVELS = ["off", "low", "medium", "high"];

const menuShell = {
  position: "absolute", left: 0, bottom: "100%", marginBottom: 8, width: 268,
  background: "var(--bg)", borderRadius: 11, boxShadow: "var(--shadow-menu)",
  padding: 4, zIndex: 30, animation: "fpop .14s ease",
};
const menuRow = {
  display: "flex", alignItems: "center", gap: 8, padding: "7px 9px",
  borderRadius: 7, cursor: "default",
};
const menuDivider = { height: 1, background: "var(--line5)", margin: "4px 8px" };

function ModelMenu({ ferry, health, onClose, onManage }) {
  const { t } = useTranslation();
  const [panel, setPanel] = useState("models");
  const models = ferry.models || [];
  const current = models.find(m =>
    m.provider === health?.provider && m.id === health?.model);
  const effort = health?.thinking || "off";

  const pick = m => {
    onClose();
    ferry.selectModel(m.provider, m.id, m.reasoning ? effort : undefined)
      .catch(ferry.reportError);
  };
  const pickEffort = level => {
    onClose();
    if (current) ferry.selectModel(current.provider, current.id, level)
      .catch(ferry.reportError);
  };

  return (
    <>
      <div onMouseDown={onClose} style={{ position: "fixed", inset: 0, zIndex: 29 }} />
      <div style={menuShell}>
        {panel === "models" ? (
          <>
            <div className="fscroll" style={{ maxHeight: 280, overflowY: "auto" }}>
              {models.map(m => (
                <div key={`${m.provider}/${m.id}`} className="hov-item"
                  onMouseDown={e => { e.preventDefault(); pick(m); }}
                  style={{ ...menuRow, alignItems: "flex-start" }}>
                  <ProviderIcon provider={m.provider} size={15} />
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)",
                      overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {m.name}</div>
                    <div style={{ fontSize: 11, color: "var(--tx4)", marginTop: 1 }}>
                      {m.provider_name}
                      {m.reasoning ? ` · ${t("askferry:model.reasoning")}` : ""}</div>
                  </div>
                  {current === m && <CheckIcon size={12} />}
                </div>
              ))}
              {!models.length && (
                <div style={{ fontSize: 11.5, color: "var(--tx5)", padding: "12px 9px",
                  lineHeight: 1.55 }}>{t("askferry:model.empty")}</div>)}
            </div>
            {current?.reasoning && (
              <>
                <div style={menuDivider} />
                <div className="hov-item" onMouseDown={e => { e.preventDefault();
                  setPanel("effort"); }} style={menuRow}>
                  <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)", flex: 1 }}>
                    {t("askferry:model.effort")}</span>
                  <span style={{ fontSize: 12, color: "var(--tx4)" }}>
                    {t(`askferry:model.effort_${effort}`)}</span>
                  <Caret size={8} dir="right" />
                </div>
              </>
            )}
            <div style={menuDivider} />
            <div className="hov-item" onMouseDown={e => { e.preventDefault(); onClose(); onManage(); }}
              style={menuRow}>
              <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)", flex: 1 }}>
                {t("askferry:model.manage")}</span>
              <Caret size={8} dir="right" />
            </div>
          </>
        ) : (
          <>
            <div className="hov-item" onMouseDown={e => { e.preventDefault(); setPanel("models"); }}
              style={menuRow}>
              <Caret size={8} dir="left" />
              <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)" }}>
                {t("askferry:model.effort")}</span>
            </div>
            <div style={menuDivider} />
            {EFFORT_LEVELS.map(level => (
              <div key={level} className="hov-item"
                onMouseDown={e => { e.preventDefault(); pickEffort(level); }} style={menuRow}>
                <span style={{ fontSize: 12.5, color: "var(--tx1)", flex: 1 }}>
                  {t(`askferry:model.effort_${level}`)}</span>
                {effort === level && <CheckIcon size={12} />}
              </div>
            ))}
          </>
        )}
      </div>
    </>
  );
}

// ----- 输入胶囊:文本域 + 底部工具条(模式/模型在左,发送在右) -----
function Composer({ ferry, text, setTextValue, taRef, mention, scanSessions,
  onPickMention, onKeyDown, onPaste, onSend, onSteer, running, mode, onOpenConfig,
  health, autoFocus, attachments, onRemoveAttachment }) {
  const { t } = useTranslation();
  const [modeOpen, setModeOpen] = useState(false);
  const [roleOpen, setRoleOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const hasContent = !!text.trim() || attachments.length > 0;
  const canSend = hasContent && ferry.available;
  const noCredential = health && health.credential === "unavailable";
  const currentModel = (ferry.models || []).find(m =>
    m.provider === health?.provider && m.id === health?.model);
  const modelLabel = currentModel?.name
    || (ferry.activeLog?.model || health?.model || "").split("/").pop();
  const effort = health?.thinking && health.thinking !== "off" ? health.thinking : null;
  // 没凭据或一个可用模型都没有时,按钮不再开下拉,而是直达设置里的提供商页
  const needsSetup = noCredential || !(ferry.models || []).length;
  useEffect(() => { if (autoFocus) taRef.current?.focus(); }, [autoFocus, taRef]);
  return (
    <div style={{ position: "relative" }}>
      {mention && (
        <MentionMenu query={mention.query} sessions={scanSessions} onPick={onPickMention} />)}
      <div className="chat-composer" style={{ padding: "12px 12px 8px 16px" }}>
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
        <textarea ref={taRef} value={text}
          rows={Math.min(8, Math.max(1, text.split("\n").length))}
          onChange={e => setTextValue(e.target.value)} onKeyDown={onKeyDown} onPaste={onPaste}
          placeholder={ferry.available ? t("askferry:composer.placeholder")
            : t("askferry:composer.desktopOnly")}
          disabled={!ferry.available}
          style={{ width: "100%", border: "none", outline: "none", resize: "none",
            background: "transparent", fontSize: 13.5, lineHeight: 1.55, color: "var(--tx1)",
            fontFamily: "inherit", padding: 0 }} />
        <div style={{ display: "flex", alignItems: "center", gap: 4, marginTop: 6 }}>
          <div style={{ position: "relative" }}>
            {modeOpen && (
              <ModeMenu mode={mode} onClose={() => setModeOpen(false)}
                onPick={k => { ferry.setMode(k); setModeOpen(false); }} />)}
            <button className="chat-ghost-btn" onClick={() => setModeOpen(v => !v)}>
              <span style={{ display: "inline-flex",
                color: mode === "auto" ? "var(--warn)" : "var(--tx3b)" }}>
                {mode === "auto" ? <AutoModeIcon size={13} /> : <ManualModeIcon size={13} />}
              </span>
              {t(mode === "auto" ? "askferry:mode.auto" : "askferry:mode.manual")}
              <Caret size={8} open={false} />
            </button>
          </div>
          <div style={{ position: "relative" }}>
            {roleOpen && (
              <RoleMenu ferry={ferry} onClose={() => setRoleOpen(false)}
                onManage={() => onOpenConfig("roles")} />)}
            <button className="chat-ghost-btn" disabled={!!ferry.activeId}
              title={ferry.activeId ? t("askferry:role.snapshotLocked") : undefined}
              onClick={() => setRoleOpen(value => !value)}>
              {(ferry.roles || []).find(role => role.id ===
                (ferry.activeId
                  ? ferry.sessions.find(session => session.session_id === ferry.activeId)?.role_id
                  : ferry.selectedRoleId))?.name || t("askferry:role.default")}
              <Caret size={8} open={false} />
            </button>
          </div>
          <div style={{ position: "relative" }}>
            {modelOpen && !needsSetup && (
              <ModelMenu ferry={ferry} health={health} onManage={() => onOpenConfig("models")}
                onClose={() => setModelOpen(false)} />)}
            <button className="chat-ghost-btn"
              onClick={() => needsSetup ? onOpenConfig() : setModelOpen(v => !v)}>
              {needsSetup ? (
                <span style={{ width: 5, height: 5, borderRadius: "50%",
                  background: "var(--warn)", flex: "none" }} />
              ) : currentModel && <ProviderIcon provider={currentModel.provider} size={13} />}
              <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {needsSetup ? t("askferry:model.setup") : (modelLabel || t("askferry:model.pick"))}</span>
              {effort && (
                <span style={{ color: "var(--tx5)", flex: "none" }}>
                  {t(`askferry:model.effort_${effort}`)}</span>)}
              <Caret size={8} open={false} />
            </button>
          </div>
          <span style={{ flex: 1 }} />
          {running && hasContent && (
            <button className="chat-ghost-btn" onClick={onSteer}>
              {t("askferry:composer.steer")}</button>
          )}
          {running && (
            <button className="chat-round-btn" title={t("askferry:composer.stop")}
              onClick={ferry.abort}><StopFillIcon /></button>
          )}
          {(!running || hasContent) && (
            <button className="chat-round-btn" disabled={!canSend}
              title={running ? t("askferry:composer.followUp") : t("askferry:composer.send")}
              onClick={onSend}><SendArrowIcon /></button>
          )}
        </div>
      </div>
    </div>
  );
}

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
            <Composer {...composerProps} autoFocus />
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
              {items.map((item, i) => (
                <ChatItem key={i} item={item} sessionId={activeId} ferry={ferry}
                  onNavigate={onNavigate} />))}
            </div>
          </div>

          {/* 底部输入区 */}
          <div style={{ flex: "none", padding: "0 24px 16px" }}>
            <div style={{ maxWidth: 680, margin: "0 auto" }}>
              <Composer {...composerProps} />
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
