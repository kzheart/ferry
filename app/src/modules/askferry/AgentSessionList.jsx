// Agent 对话列表：悬浮显示置顶、删除和更多操作；更多菜单承载低频动作。
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useFerryRuntime } from "../../shared/capabilities/ferryRuntime.jsx";
import { MoreDots, PinIcon, PlusIcon, Spinner, TrashIcon } from "../../shared/ui/icons.jsx";
import { writeClipboardText } from "../../platform/desktop/client.js";
import { fmtTime } from "../browser/public.js";

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

// 提醒徽标的颜色与提示：待审批用警告色并放大一圈，让它在一列圆点里跳出来
const ATTENTION_STYLE = {
  approval: { color: "var(--warn)", size: 8, key: "attentionApproval" },
  error: { color: "var(--err)", size: 7, key: "attentionError" },
  unread: { color: "var(--accent)", size: 7, key: "attentionUnread" },
};

const AgentSessionRow = memo(function AgentSessionRow({ session, active, editing,
  onOpen, onPin, onDelete, onStartRename, onSubmitRename, onCancelRename }) {
  const { t } = useTranslation();
  const [menu, setMenu] = useState(null);
  const running = session.status === "running";
  const title = session.title || t("askferry:chat.untitled");
  const action = callback => event => { event.stopPropagation(); callback(session); };
  const badge = ATTENTION_STYLE[session.attention];
  // Enter/Esc 已经了结这次重命名后,紧随的 blur 不能再触发一次提交
  const settled = useRef(false);

  if (editing) {
    return (
      <div className="lib-row" style={{ display: "flex", alignItems: "center", padding: "5px 8px",
        height: 30, borderRadius: 6, background: "var(--acc-soft2)" }}>
        <input autoFocus defaultValue={session.title || ""}
          placeholder={t("askferry:pane.renamePlaceholder")}
          onFocus={event => { settled.current = false; event.target.select(); }}
          // 失焦即提交(对齐 Finder):点向别处不该丢掉刚输入的名字
          onBlur={event => {
            if (settled.current) return;
            settled.current = true;
            onSubmitRename(session, event.currentTarget.value);
          }}
          onClick={event => event.stopPropagation()}
          onKeyDown={event => {
            event.stopPropagation();
            if (event.key === "Enter") {
              settled.current = true;
              onSubmitRename(session, event.currentTarget.value);
            } else if (event.key === "Escape") {
              settled.current = true;
              onCancelRename();
            }
          }}
          style={{ flex: 1, minWidth: 0, height: 20, border: "none", outline: "none",
            background: "transparent", color: "var(--tx1)", fontSize: 12, padding: 0 }} />
      </div>
    );
  }

  return (
    <div onClick={() => onOpen(session.session_id)}
      onDoubleClick={event => {
        if (!event.target.closest(".row-act")) onStartRename(session);
      }}
      className={active ? "lib-row" : "lib-row hov-item"}
      style={{ display: "flex", gap: 8, alignItems: "center", padding: "5px 8px", height: 30,
        borderRadius: 6, cursor: "default", transition: "background .12s ease",
        background: active ? "var(--acc-soft2)" : "transparent" }}>
      {running
        ? <Spinner size={12} />
        : badge
          ? <span title={t(`askferry:pane.${badge.key}`)}
              style={{ width: badge.size, height: badge.size, borderRadius: "50%",
                background: badge.color, flex: "none" }} />
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
      {menu && <MoreMenu item={session} anchor={menu} onClose={() => setMenu(null)} onRename={onStartRename} />}
    </div>
  );
});

// 按更新时间切段：传入的 sessions 已是「置顶优先 + 时间倒序」，这里只负责插分组头
function groupSessions(sessions) {
  const startOfToday = new Date();
  startOfToday.setHours(0, 0, 0, 0);
  const today = startOfToday.getTime();
  const yesterday = today - 86400000;
  const last7 = today - 6 * 86400000;
  const buckets = new Map();
  const push = (key, session) => {
    const rows = buckets.get(key);
    if (rows) rows.push(session);
    else buckets.set(key, [session]);
  };
  for (const session of sessions) {
    if (session.pinned) { push("pinned", session); continue; }
    const at = session.updated_at ? Date.parse(session.updated_at) : NaN;
    if (!Number.isFinite(at) || at < last7) push("earlier", session);
    else if (at >= today) push("today", session);
    else if (at >= yesterday) push("yesterday", session);
    else push("last7", session);
  }
  return ["pinned", "today", "yesterday", "last7", "earlier"]
    .filter(key => buckets.has(key))
    .map(key => ({ key, rows: buckets.get(key) }));
}

const GROUP_LABEL = {
  pinned: "askferry:pane.pinnedGroup",
  today: "common:bucket.today",
  yesterday: "common:bucket.yesterday",
  last7: "common:bucket.last7",
  earlier: "common:bucket.earlier",
};

// 打开/新建/置顶/删除都是 Ferry Runtime 上的动作,自己取;列表内容由主壳决定
// (经过筛选排序),仍走 props。重命名走行内编辑:双击行或「更多」菜单进入。
export default function AgentSessionList({ sessions }) {
  const { t } = useTranslation();
  const ferry = useFerryRuntime();
  const { activeId, openSession: onOpen, newChat: onNew } = ferry;
  const [editingId, setEditingId] = useState(null);
  const onPin = session =>
    ferry.pin(session.session_id, !session.pinned).catch(ferry.reportError);
  const onDelete = session =>
    ferry.deleteSession(session.session_id).catch(ferry.reportError);
  const onSubmitRename = (session, value) => {
    setEditingId(null);
    const title = value.trim();
    if (!title || title === session.title) return;
    ferry.rename(session.session_id, title).catch(ferry.reportError);
  };

  const groups = useMemo(() => groupSessions(sessions), [sessions]);

  // ↑/↓ 在当前可见顺序里换会话;分组只是切段,展开后的顺序与 sessions 一致
  const navRef = useRef({ sessions, activeId, onOpen, editing: false });
  navRef.current = { sessions, activeId, onOpen, editing: editingId !== null };
  useEffect(() => {
    const onKeyDown = event => {
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      const el = document.activeElement;
      if (el && (["INPUT", "TEXTAREA"].includes(el.tagName) || el.isContentEditable)) return;
      const { sessions: list, activeId: current, onOpen: open, editing } = navRef.current;
      if (editing || !list.length) return;
      event.preventDefault();
      const index = list.findIndex(s => s.session_id === current);
      const step = event.key === "ArrowDown" ? 1 : -1;
      const next = index < 0
        ? 0
        : Math.max(0, Math.min(list.length - 1, index + step));
      open(list[next].session_id);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

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
      {groups.map(group => (
        <div key={group.key} style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 5, padding: "0 8px",
            height: 24, marginTop: 3 }}>
            <span style={{ fontSize: 11, fontWeight: 600, color: "var(--tx3)" }}>
              {t(GROUP_LABEL[group.key])}</span>
            <span style={{ fontSize: 11, color: "var(--tx5)" }}>· {group.rows.length}</span>
          </div>
          {group.rows.map(session => (
            <AgentSessionRow key={session.session_id} session={session}
              active={session.session_id === activeId}
              editing={session.session_id === editingId}
              onOpen={onOpen} onPin={onPin} onDelete={onDelete}
              onStartRename={item => setEditingId(item.session_id)}
              onSubmitRename={onSubmitRename}
              onCancelRename={() => setEditingId(null)} />
          ))}
        </div>
      ))}
    </div>
  );
}
