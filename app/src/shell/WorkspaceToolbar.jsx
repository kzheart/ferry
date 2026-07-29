// 标题栏工具条:侧栏开关 + 当前工作区的动作按钮 + 可拖拽留白。
import { useTranslation } from "react-i18next";
import { SidebarIcon } from "../shared/ui/icons.jsx";

export function WorkspaceToolbar({
  paneAvailable,
  collapsed,
  onToggleCollapsed,
}) {
  const { t } = useTranslation();
  return (
    <>
      {/* 侧栏开关常驻工具栏(macOS 惯例):无资源栏的视图置灰禁用,避免切视图时按钮突然消失 */}
      <button
        className={paneAvailable ? "hov" : undefined}
        disabled={!paneAvailable}
        onClick={onToggleCollapsed}
        title={
          collapsed ? t("app:titlebar.expand") : t("app:titlebar.collapse")
        }
        style={{
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
          opacity: paneAvailable ? 1 : 0.35,
        }}
      >
        <SidebarIcon />
      </button>
      <div data-tauri-drag-region style={{ flex: 1, alignSelf: "stretch" }} />
    </>
  );
}
