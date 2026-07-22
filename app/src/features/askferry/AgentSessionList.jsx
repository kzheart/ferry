// Ask Ferry 对话列表(资源栏):新建 + 会话行(标题/状态/时间)
import { useTranslation } from "react-i18next";
import { PlusIcon, Spinner } from "../../components/ui/icons.jsx";
import { fmtTime } from "../../domain/sessions/sessionModel.js";

export default function AgentSessionList({ sessions, titles, activeId, onOpen, onNew }) {
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
      {sessions.map(s => {
        const on = s.session_id === activeId;
        return (
          <div key={s.session_id} onClick={() => onOpen(s.session_id)}
            className={on ? "lib-row" : "lib-row hov-item"}
            style={{ display: "flex", gap: 8, alignItems: "center", padding: "5px 8px", height: 30,
              borderRadius: 6, cursor: "default",
              background: on ? "var(--acc-soft2)" : "transparent" }}>
            {s.status === "running"
              ? <Spinner size={12} />
              : <span style={{ width: 6, height: 6, borderRadius: "50%",
                  background: "var(--line-strong)", flex: "none" }} />}
            <span style={{ fontSize: 12, color: "var(--tx1)", whiteSpace: "nowrap",
              overflow: "hidden", textOverflow: "ellipsis", flex: 1, minWidth: 0 }}>
              {titles[s.session_id] || t("askferry:chat.untitled")}</span>
            <span style={{ fontSize: 10, color: "var(--tx5)", flex: "none" }}>
              {s.updated_at ? fmtTime(Date.parse(s.updated_at), t) : ""}</span>
          </div>
        );
      })}
    </div>
  );
}
