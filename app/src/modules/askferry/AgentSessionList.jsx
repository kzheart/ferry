// Agent 对话列表：悬浮显示置顶、删除和更多操作；更多菜单承载低频动作。
import { memo, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { MoreDots, PinIcon, PlusIcon, Spinner, TrashIcon } from "../../shared/ui/icons.jsx";
import { writeClipboardText } from "../../platform/desktop/client.js";
import { fmtTime } from "../browser/sessionModel.js";

function MoreMenu({ item, anchor, onClose, onRename }) {
  const { t } = useTranslation();
  useEffect(() => {
    const onKeyDown = event => { if (event.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);
  if (!anchor) return null;
  const width = 174;
  const left = Math.max(8, Math.min(anchor.right - width, window.innerWidth - width - 8));
  const top = Math.min(anchor.bottom + 5, window.innerHeight - 82);
  return createPortal(<>
    <div onMouseDown={onClose} style={{ position: "fixed", inset: 0, zIndex: 60 }} />
    <div style={{ position: "fixed", top, left, width, zIndex: 61, padding: 5,
      background: "var(--bg)", borderRadius: 9, boxShadow: "var(--shadow-menu)", animation: "fpop .14s ease" }}>
      <button className="hov-item" onClick={() => { onClose(); onRename(item); }}
        style={{ width: "100%", height: 30, display: "flex", alignItems: "center", border: "none",
          borderRadius: 6, padding: "0 9px", background: "transparent", color: "var(--tx2)",
          cursor: "default", fontSize: 12, textAlign: "left" }}>
        {t("askferry:pane.rename")}
      </button>
      <button className="hov-item" onClick={() => {
        writeClipboardText(item.session_id).catch(() => {});
        onClose();
      }} style={{ width: "100%", height: 30, display: "flex", alignItems: "center", border: "none",
        borderRadius: 6, padding: "0 9px", background: "transparent", color: "var(--tx2)",
        cursor: "default", fontSize: 12, textAlign: "left" }}>
        {t("askferry:pane.copyId")}
      </button>
    </div>
  </>, document.body);
}

const AgentSessionRow = memo(function AgentSessionRow({ session, active, onOpen, onPin, onDelete, onRename }) {
  const { t } = useTranslation();
  const [menu, setMenu] = useState(null);
  const running = session.status === "running";
  const title = session.title || t("askferry:chat.untitled");
  const action = callback => event => { event.stopPropagation(); callback(session); };
  return (
    <div onClick={() => onOpen(session.session_id)}
      className={active ? "lib-row" : "lib-row hov-item"}
      style={{ display: "flex", gap: 8, alignItems: "center", padding: "5px 8px", height: 30,
        borderRadius: 6, cursor: "default", transition: "background .12s ease",
        background: active ? "var(--acc-soft2)" : "transparent" }}>
      {running
        ? <Spinner size={12} />
        : <span style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--line-strong)", flex: "none" }} />}
      <span title={title} style={{ fontSize: 12, color: "var(--tx1)", whiteSpace: "nowrap",
        overflow: "hidden", textOverflow: "ellipsis", flex: 1, minWidth: 0 }}>{title}</span>
      {session.pinned && <span className="row-meta" style={{ display: "inline-flex", color: "var(--accent)" }}><PinIcon filled /></span>}
      <span className="row-meta" style={{ fontSize: 10, color: "var(--tx5)", flex: "none" }}>
        {session.updated_at ? fmtTime(Date.parse(session.updated_at), t) : ""}
      </span>
      <span className="row-act" style={{ gap: 1, flex: "none" }}>
        <button className="row-act-btn" onClick={action(onPin)}
          title={session.pinned ? t("askferry:pane.unpin") : t("askferry:pane.pin")}
          style={session.pinned ? { color: "var(--accent)" } : undefined}>
          <PinIcon filled={session.pinned} />
        </button>
        <button className="row-act-btn row-act-danger" onClick={action(onDelete)} disabled={running}
          title={running ? t("askferry:pane.stopBeforeDelete") : t("askferry:pane.delete")}>
          <TrashIcon size={13} />
        </button>
        <button className="row-act-btn" onClick={event => {
          event.stopPropagation();
          setMenu({ right: event.currentTarget.getBoundingClientRect().right,
            bottom: event.currentTarget.getBoundingClientRect().bottom });
        }} title={t("askferry:pane.more")}><MoreDots /></button>
      </span>
      {menu && <MoreMenu item={session} anchor={menu} onClose={() => setMenu(null)} onRename={onRename} />}
    </div>
  );
});

export default function AgentSessionList({ sessions, activeId, onOpen, onNew, onPin, onDelete, onRename }) {
  const { t } = useTranslation();
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <div className="hov-item" onClick={onNew}
        style={{ display: "flex", alignItems: "center", gap: 8, padding: "5px 8px", height: 30,
          borderRadius: 6, cursor: "default", margin: "0 0 6px", color: "var(--tx2)" }}>
        <PlusIcon size={12} />
        <span style={{ fontSize: 12, fontWeight: 500 }}>{t("askferry:pane.newChat")}</span>
      </div>
      {sessions.length === 0 && (
        <div style={{ padding: "26px 12px", textAlign: "center", color: "var(--tx5)", fontSize: 12 }}>
          {t("askferry:pane.empty")}</div>)}
      {sessions.map(session => <AgentSessionRow key={session.session_id} session={session}
        active={session.session_id === activeId} onOpen={onOpen} onPin={onPin}
        onDelete={onDelete} onRename={onRename} />)}
    </div>
  );
}
