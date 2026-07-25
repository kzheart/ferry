// 角色页左栏:角色清单 + 新建入口 + 导入导出菜单。
import { useTranslation } from "react-i18next";
import { RoleAvatar } from "../../shared/ui/icons.jsx";

// 只放"整体"层面的动作:导出单个角色跟着角色标题走,不在这里重复一遍
function TransferMenu({ onExportAll, onImport, onClose }) {
  const { t } = useTranslation();
  const item = (label, onClick) => (
    <button type="button" className="hov-item"
      onMouseDown={event => { event.preventDefault(); onClose(); onClick(); }}
      style={{ display: "block", width: "100%", textAlign: "left", padding: "7px 9px",
        border: "none", borderRadius: 7, background: "transparent", fontFamily: "inherit",
        fontSize: 12, color: "var(--tx1)", cursor: "default" }}>
      {label}</button>
  );
  return (
    <>
      <div onMouseDown={onClose} style={{ position: "fixed", inset: 0, zIndex: 69 }} />
      <div style={{ position: "absolute", left: 0, bottom: "100%", marginBottom: 6, width: 186,
        background: "var(--surface)", borderRadius: 11, boxShadow: "var(--shadow-menu)",
        padding: 5, zIndex: 70, animation: "fpop .14s ease" }}>
        {item(t("settings:roles.exportAll"), onExportAll)}
        {item(t("settings:roles.import"), onImport)}
      </div>
    </>
  );
}

export default function RoleList({
  roles, selectedId, creating, draft, busy, transferOpen,
  onSelect, onCreate, onToggleTransfer, transfer,
}) {
  const { t } = useTranslation();
  return (
    <div style={{ width: 188, flex: "none", borderRight: "1px solid var(--line4)",
      display: "flex", flexDirection: "column", minHeight: 0 }}>
      <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "10px 8px",
        display: "flex", flexDirection: "column", gap: 2 }}>
        {roles.map(role => {
          const on = !creating && selectedId === role.id;
          return (
            <button key={role.id} className={on ? undefined : "hov-item"}
              onClick={() => onSelect(role.id)}
              style={{ display: "flex", alignItems: "center", gap: 9, border: "none",
                borderRadius: 8, padding: "7px 8px", textAlign: "left", cursor: "default",
                fontFamily: "inherit", background: on ? "var(--seg-on)" : "transparent" }}>
              <RoleAvatar icon={role.icon} color={role.color} size={28} />
              <span style={{ minWidth: 0, flex: 1 }}>
                <span style={{ display: "block", fontSize: 12.5, whiteSpace: "nowrap",
                  overflow: "hidden", textOverflow: "ellipsis",
                  fontWeight: on ? 650 : 600, color: on ? "var(--tx1)" : "var(--tx2b)" }}>
                  {role.name}</span>
                <span style={{ display: "block", marginTop: 1, fontSize: 10.5,
                  color: "var(--tx5)", whiteSpace: "nowrap", overflow: "hidden",
                  textOverflow: "ellipsis" }}>
                  {role.builtin ? t("settings:roles.builtin")
                    : t("settings:roles.toolCount", { n: role.tools.length })}</span>
              </span>
            </button>
          );
        })}
        {creating && (
          <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "7px 8px",
            borderRadius: 8, background: "var(--seg-on)" }}>
            <RoleAvatar icon={draft.icon} color={draft.color} size={28} />
            <span style={{ fontSize: 12.5, fontWeight: 650, color: "var(--tx1)",
              minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {draft.name || t("settings:roles.create")}</span>
          </div>
        )}
      </div>
      <div style={{ position: "relative", flex: "none", display: "flex", gap: 4,
        padding: "7px 10px", borderTop: "1px solid var(--line4)" }}>
        <button className="hov-item" onClick={onCreate} disabled={busy}
          style={{ display: "flex", alignItems: "center", gap: 7, flex: 1, height: 28,
            padding: "0 8px", border: "none", borderRadius: 7, background: "transparent",
            color: "var(--tx3b)", fontFamily: "inherit", fontSize: 12, fontWeight: 600,
            cursor: "default" }}>
          <svg viewBox="0 0 16 16" aria-hidden style={{ width: 13, height: 13 }}>
            <path d="M8 2.8v10.4M2.8 8h10.4" fill="none" stroke="currentColor"
              strokeWidth="1.6" strokeLinecap="round" />
          </svg>
          {t("settings:roles.create")}
        </button>
        {transferOpen && (
          <TransferMenu {...transfer} onClose={() => onToggleTransfer(false)} />)}
        <button className="hov" title={t("settings:roles.transfer")} disabled={busy}
          onClick={() => onToggleTransfer(!transferOpen)}
          style={{ width: 26, height: 28, border: "none", borderRadius: 7, flex: "none",
            background: "transparent", color: "var(--tx3b)", cursor: "default",
            display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
          <svg viewBox="0 0 16 16" aria-hidden style={{ width: 13, height: 13 }}>
            <circle cx="3.4" cy="8" r="1.35" fill="currentColor" />
            <circle cx="8" cy="8" r="1.35" fill="currentColor" />
            <circle cx="12.6" cy="8" r="1.35" fill="currentColor" />
          </svg>
        </button>
      </div>
    </div>
  );
}
