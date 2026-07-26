import { ACCENT } from "../shared/ui/toolDisplay.js";
import { useFerryRuntime } from "../shared/capabilities/ferryRuntime.jsx";
import { RailGlyph, RescanIcon, Spinner } from "../shared/ui/icons.jsx";

// Agent 图标上的角标:后台会话有事要人处理时是警告色实点,只是在跑用弱色。
// 状态从 Runtime 直接取,不经 props——它与主壳的布局参数无关。
function useAgentRailDot() {
  const { sessions } = useFerryRuntime();
  if (sessions.some(s => s.attention)) return "var(--warn)";
  if (sessions.some(s => s.status === "running")) return "var(--tx5)";
  return null;
}

export function AppRail({
  railOnly,
  resizing,
  items,
  activeView,
  draggingKey,
  dropTarget,
  scanning,
  settingsOpen,
  scanningLabel,
  rescanLabel,
  settingsLabel,
  onSelect,
  onRescan,
  onToggleSettings,
  onEnter,
  onLeave,
  pointerHandlers,
}) {
  const agentDot = useAgentRailDot();
  return (
    <div style={{ width: railOnly ? 80 : 56, flex: "none", background: "var(--pane)",
      position: "relative", display: "flex", flexDirection: "column", alignItems: "center",
      padding: "0 0 12px", gap: 4, zIndex: 5,
      transition: resizing ? "none" : "width .2s ease-out" }}>
      {railOnly && (
        <div style={{ position: "absolute", right: 0, top: 44, bottom: 0, width: 1,
          background: "var(--line)", pointerEvents: "none" }} />
      )}
      <div data-tauri-drag-region style={{ height: 44, alignSelf: "stretch", flex: "none" }} />
      {items.map(item => {
        const active = activeView === item.key;
        const dropBefore = dropTarget?.key === item.key &&
          dropTarget.position === "before" && draggingKey !== item.key;
        const dropAfter = dropTarget?.key === item.key &&
          dropTarget.position === "after" && draggingKey !== item.key;
        return (
          <button key={item.key} className="hov-rail"
            data-rail-key={item.key}
            data-guide={item.key === "library" ? "rail" : undefined}
            onMouseEnter={event => onEnter(item.label, event)} onMouseLeave={onLeave}
            onPointerDown={pointerHandlers.down} onPointerMove={pointerHandlers.move}
            onPointerUp={pointerHandlers.up} onPointerCancel={pointerHandlers.cancel}
            onClick={() => onSelect(item.key)}
            style={{ width: 40, height: 40, border: "none", borderRadius: 8, position: "relative",
              background: active ? "var(--acc-soft2)" : "transparent", display: "flex", alignItems: "center",
              justifyContent: "center", cursor: "default",
              touchAction: "none", opacity: draggingKey === item.key ? .48 : 1,
              transform: draggingKey === item.key ? "scale(.9)" : "none",
              boxShadow: dropBefore ? `0 -2px 0 ${ACCENT}` : dropAfter ? `0 2px 0 ${ACCENT}` : "none",
              transition: "background .12s ease, transform .12s ease, opacity .12s ease" }}>
            <RailGlyph name={item.key} color={active ? ACCENT : "var(--tx4b)"} />
            {item.key === "askferry" && agentDot && (
              <span style={{ position: "absolute", top: 8, right: 8, width: 6, height: 6,
                borderRadius: "50%", background: agentDot }} />
            )}
          </button>
        );
      })}
      <div style={{ flex: 1 }} />
      <button className="hov-rail"
        onMouseEnter={event => onEnter(scanning ? scanningLabel : rescanLabel, event)}
        onMouseLeave={onLeave}
        disabled={scanning}
        onClick={onRescan}
        style={{ width: 40, height: 40, border: "none", borderRadius: 8,
          background: "transparent", display: "flex", alignItems: "center",
          justifyContent: "center", cursor: "default", transition: "background .12s ease",
          color: "var(--tx4b)" }}>
        {scanning ? <Spinner size={18} /> : <RescanIcon size={18} color="var(--tx4b)" />}
      </button>
      <button className="hov-rail"
        onMouseEnter={event => onEnter(settingsLabel, event)} onMouseLeave={onLeave}
        onClick={onToggleSettings}
        style={{ width: 40, height: 40, border: "none", borderRadius: 8,
          background: settingsOpen ? "var(--acc-soft2)" : "transparent", display: "flex",
          alignItems: "center", justifyContent: "center", cursor: "default",
          transition: "background .12s ease" }}>
        <RailGlyph name="settings" color={settingsOpen ? ACCENT : "var(--tx4b)"} />
      </button>
    </div>
  );
}
