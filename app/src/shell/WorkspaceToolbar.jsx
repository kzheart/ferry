// 标题栏工具条:导航栏开关 + 资源栏开关 + 当前工作区的动作按钮 + 可拖拽留白。
import { useTranslation } from "react-i18next";
import { NavToggleIcon, SidebarIcon } from "../shared/ui/icons.jsx";

const btn = {
  width: 28,
  height: 26,
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  background: "transparent",
  border: "none",
  borderRadius: 6,
  cursor: "default",
  color: "var(--tx3b)",
};

export function WorkspaceToolbar({
  paneAvailable,
  collapsed,
  onToggleCollapsed,
  navCollapsed,
  onToggleNav,
}) {
  const { t } = useTranslation();
  return (
    <>
      {/* 导航栏开关:⌘⇧S 之外的那条明路,不然折叠了就没人知道怎么展开 */}
      <button
        className="hov"
        onClick={onToggleNav}
        title={navCollapsed ? t("app:nav.expand") : t("app:nav.collapse")}
        style={btn}
      >
        <NavToggleIcon />
      </button>
      {/* 侧栏开关常驻工具栏(macOS 惯例):无资源栏的视图置灰禁用,避免切视图时按钮突然消失 */}
      <button
        className={paneAvailable ? "hov" : undefined}
        disabled={!paneAvailable}
        onClick={onToggleCollapsed}
        title={
          collapsed ? t("app:titlebar.expand") : t("app:titlebar.collapse")
        }
        style={{ ...btn, opacity: paneAvailable ? 1 : 0.35 }}
      >
        <SidebarIcon />
      </button>
      <div data-tauri-drag-region style={{ flex: 1, alignSelf: "stretch" }} />
    </>
  );
}
