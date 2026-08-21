import { ACCENT } from "../shared/ui/toolDisplay.js";

export function AppShell({
  rail,
  resourcePane,
  showDivider,
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
        <div onMouseDown={onResizeStart} onDoubleClick={onResizeReset}
          title={dividerTitle}
          style={{ width: 9, flex: "none", cursor: "col-resize", position: "relative",
            background: resizing ? "var(--acc-soft2)" : "var(--bg)", zIndex: 6 }}>
          <div style={{ position: "absolute", left: 0, top: 0, bottom: 0, width: 1,
            background: resizing ? ACCENT : "var(--line)" }} />
        </div>
      )}
      {/* 主区最小宽度:导航栏 200 + 资源栏 232 之外剩下的都归它,不允许被挤没 */}
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", background: "var(--bg)" }}>
        <div data-tauri-drag-region style={{ height: 44, flex: "none", display: "flex", alignItems: "center",
          gap: 12, padding: "0 12px" }}>
          {toolbar}
        </div>
        <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
          {children}
        </div>
      </div>
    </>
  );
}
