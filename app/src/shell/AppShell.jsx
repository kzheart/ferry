import { ACCENT } from "../shared/ui/toolDisplay.js";
import { OVERLAY_TITLEBAR } from "../shared/ui/platform.js";

export function AppShell({
  rail,
  resourcePane,
  sidebarCollapsed,
  showDivider,
  dividerCollapsed,
  resizing,
  onResizeStart,
  onResizeReset,
  dividerTitle,
  toolbar,
  children,
}) {
  return (
    <>
      {rail}
      {resourcePane}
      {showDivider && (
        <div onMouseDown={dividerCollapsed ? undefined : onResizeStart}
          onDoubleClick={dividerCollapsed ? undefined : onResizeReset}
          title={dividerCollapsed ? undefined : dividerTitle}
          style={{ width: dividerCollapsed ? 0 : 9, flex: "none", overflow: "hidden",
            cursor: dividerCollapsed ? "default" : "col-resize", position: "relative",
            background: resizing ? "var(--acc-soft2)" : "var(--bg)", zIndex: 6,
            transition: resizing ? "none" : "width .2s ease-out" }}>
          <div style={{ position: "absolute", left: 0, top: 0, bottom: 0, width: 1,
            background: resizing ? ACCENT : "var(--line)" }} />
        </div>
      )}
      {/* 主区最小宽度:导航栏 200 + 资源栏 232 之外剩下的都归它,不允许被挤没 */}
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", background: "var(--bg)" }}>
        {/* 侧栏收起后主区顶到窗口左上角,红绿灯就压在工具条上,得给它留出位置 */}
        <div data-tauri-drag-region style={{ height: 44, flex: "none", display: "flex", alignItems: "center",
          gap: 12, padding: sidebarCollapsed && OVERLAY_TITLEBAR ? "0 12px 0 78px" : "0 12px",
          transition: "padding .2s ease-out" }}>
          {toolbar}
        </div>
        <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
          {children}
        </div>
      </div>
    </>
  );
}
