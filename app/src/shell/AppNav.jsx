// Wake 式导航栏:上半是应用导航(可拖拽排序),下半是会话库的「范围」——
// 全部 / 置顶 / 各 Agent / 各项目 / 各标签,常驻可见、带计数、单选互斥。
// 只有「在」与「不在」两态:收起时连同资源栏一起退场,不留没有文字的图标轨。
import { useEffect, useState } from "react";
import { ACCENT } from "../shared/ui/toolDisplay.js";
import { useFerryRuntime } from "../shared/capabilities/ferryRuntime.jsx";
import { Caret, PinIcon, RailGlyph, RescanIcon, Spinner, ToolIcon } from "../shared/ui/icons.jsx";
import { useDensityMetrics } from "../shared/ui/density.js";
import { TITLEBAR_INSET } from "../shared/ui/platform.js";

// 宽度由密度变量决定(standard 240 / compact 208)
const NAV_WIDTH = "var(--nav-w)";
const SECTIONS_KEY = "ferry-nav-sections";

// Agent 图标上的角标:后台会话有事要人处理时是警告色实点,只是在跑用弱色。
// 状态从 Runtime 直接取,不经 props——它与主壳的布局参数无关。
function useAgentRailDot() {
  const { sessions } = useFerryRuntime();
  if (sessions.some(s => s.attention)) return "var(--warn)";
  if (sessions.some(s => s.status === "running")) return "var(--tx5)";
  return null;
}

// 分区折叠状态:记住的是「哪些分区被收起来了」,新出现的分区默认展开
function useNavSections() {
  const [collapsed, setCollapsed] = useState(() => {
    try {
      const stored = JSON.parse(localStorage.getItem(SECTIONS_KEY) || "null");
      return stored && typeof stored === "object" ? stored : {};
    } catch {
      return {};
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(SECTIONS_KEY, JSON.stringify(collapsed));
    } catch {
      // 存不进去就只在本次会话里生效
    }
  }, [collapsed]);
  return {
    isOpen: key => !collapsed[key],
    toggle: key => setCollapsed(value => ({ ...value, [key]: !value[key] })),
  };
}

export const FolderGlyph = ({ size = 17 }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden
    style={{ flex: "none", color: "currentColor" }}>
    <path d="M2 4.2c0-.7.5-1.2 1.2-1.2h2.4l1.3 1.5h5.9c.7 0 1.2.5 1.2 1.2v5.6c0 .7-.5 1.2-1.2 1.2H3.2c-.7 0-1.2-.5-1.2-1.2V4.2z"
      stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" />
  </svg>
);

export const TagGlyph = ({ size = 17 }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden
    style={{ flex: "none", color: "currentColor" }}>
    <path d="M2.5 2.5h5l6 6-5 5-6-6v-5z" stroke="currentColor" strokeWidth="1.2"
      strokeLinejoin="round" />
    <circle cx="5.4" cy="5.4" r="1" fill="currentColor" />
  </svg>
);

// 导航栏的一行:图标 + 文字 + 右侧计数。选中态沿用列表行的整块填充,不描边。
export function NavRow({
  icon, label, count, active, title, indent, onClick, dataKey, guide,
  onEnter, onLeave, pointerHandlers, dragging, dropBefore, dropAfter, badge,
  draggable, onDragStart, onDragOver, onDrop, onDragEnd, onContextMenu,
}) {
  return (
    <button type="button" className="nav-row"
      data-rail-key={dataKey}
      data-guide={guide}
      aria-current={active ? "true" : undefined}
      title={title}
      onMouseEnter={onEnter} onMouseLeave={onLeave}
      onPointerDown={pointerHandlers?.down} onPointerMove={pointerHandlers?.move}
      onPointerUp={pointerHandlers?.up} onPointerCancel={pointerHandlers?.cancel}
      onClick={onClick}
      draggable={draggable || undefined}
      onDragStart={onDragStart} onDragOver={onDragOver} onDrop={onDrop} onDragEnd={onDragEnd}
      onContextMenu={onContextMenu}
      style={{ height: "var(--nav-row-h)", display: "flex", alignItems: "center", gap: 8,
        padding: indent ? "0 10px 0 20px" : "0 10px", border: "none", borderRadius: 6,
        width: "100%", position: "relative", textAlign: "left", fontFamily: "inherit",
        fontSize: "var(--fs-body)", cursor: "default", touchAction: "none",
        background: active ? "var(--acc-soft2)" : "transparent",
        color: active ? "var(--tx1)" : "var(--tx2)",
        fontWeight: active ? 500 : 400,
        opacity: dragging ? .48 : 1,
        boxShadow: dropBefore ? `0 -2px 0 ${ACCENT}` : dropAfter ? `0 2px 0 ${ACCENT}` : "none",
        transition: "background .12s ease, color .12s ease, opacity .12s ease" }}>
      <span style={{ flex: "none", display: "flex", alignItems: "center",
        transition: "color .12s ease",
        color: active ? "var(--tx1)" : "var(--tx4b)" }}>{icon}</span>
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis",
        whiteSpace: "nowrap" }}>{label}</span>
      {badge}
      {count != null && (
        <span className="mono tnum nav-count" style={{ flex: "none", fontSize: 12,
          color: "var(--tx5)", textAlign: "right",
          transition: "color .12s ease, opacity .12s ease" }}>
          {count}
        </span>
      )}
    </button>
  );
}

function NavSection({ label, open, onToggle }) {
  return (
    <button type="button" className="nav-section" onClick={onToggle}
      aria-expanded={open}
      style={{ height: 22, marginTop: "var(--section-gap)", display: "flex",
        alignItems: "center", gap: 4,
        padding: "0 10px", border: "none", background: "transparent", width: "100%",
        color: "var(--tx5)", fontFamily: "inherit", fontSize: 11, fontWeight: 600,
        letterSpacing: ".05em", textTransform: "uppercase", cursor: "default" }}>
      <Caret open={open} size={8} />
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis",
        whiteSpace: "nowrap", textAlign: "left" }}>{label}</span>
    </button>
  );
}

export function AppNav({
  collapsed,
  items,
  activeView,
  draggingKey,
  dropTarget,
  scanning,
  settingsOpen,
  labels,
  scope,
  scopeCounts,
  favoriteProjects = [],
  onReorderFavorite = () => {},
  onSelectScope,
  onSelect,
  onRescan,
  onToggleSettings,
  settingsBadge,
  pointerHandlers,
}) {
  const agentDot = useAgentRailDot();
  const sections = useNavSections();
  const metrics = useDensityMetrics();
  // 收藏行的拖拽排序:HTML5 拖放就够用了,不必复用页面项那套指针拖拽
  const [dragDir, setDragDir] = useState(null);
  const [dropAt, setDropAt] = useState(null);
  const scopeActive = key => activeView === "library" && scope.kind === key;
  const dropSlot = key => ({
    dragging: draggingKey === key,
    dropBefore: dropTarget?.key === key && dropTarget.position === "before"
      && draggingKey !== key,
    dropAfter: dropTarget?.key === key && dropTarget.position === "after"
      && draggingKey !== key,
  });
  const pageCount = key => (key === "library" ? scopeCounts.total : null);

  return (
    // 收起是把宽度收成 0 而不是卸载:内层始终按原宽排版,整栏才能平滑地滑进滑出
    <div data-guide="rail" aria-hidden={collapsed || undefined}
      style={{ width: collapsed ? 0 : NAV_WIDTH, flex: "none", overflow: "hidden",
        background: "var(--nav)", position: "relative", zIndex: 5, minHeight: 0,
        transition: "width .2s ease-out" }}>
      {/* 右侧分隔线:导航栏与资源栏同族底色,没有这条线两块会糊成一片 */}
      <div style={{ position: "absolute", right: 0, top: 0, bottom: 0, width: 1,
        background: "var(--line)", pointerEvents: "none" }} />
      {/* 收起后内容还在 DOM 里,visibility 延到滑动结束再切,免得它还能被 Tab 聚焦 */}
      <div style={{ width: NAV_WIDTH, height: "100%", display: "flex",
        flexDirection: "column", alignItems: "stretch", padding: "0 8px 10px", gap: 1,
        minHeight: 0, opacity: collapsed ? 0 : 1,
        visibility: collapsed ? "hidden" : "visible",
        transition: collapsed
          ? "opacity .12s ease, visibility 0s linear .2s"
          : "opacity .12s ease" }}>
      <div data-tauri-drag-region style={{ height: TITLEBAR_INSET, alignSelf: "stretch", flex: "none" }} />

      <div className="fscroll nav-scroll" style={{ flex: 1, minHeight: 0, overflowY: "auto",
        display: "flex", flexDirection: "column", alignItems: "stretch", gap: 1 }}>
        {items.map(item => {
          // 选了某个范围之后「会话」就不再是当前项了——高亮的是那条范围行
          const active = activeView === item.key
            && !(item.key === "library" && scope.kind !== "all");
          const badge = item.key === "askferry" && agentDot
            ? <span style={{ flex: "none", width: 6, height: 6, borderRadius: "50%",
                background: agentDot }} />
            : null;
          return (
            <div key={item.key}>
              <NavRow dataKey={item.key} guide={`rail-${item.key}`}
                icon={<RailGlyph name={item.key} size={metrics.navIcon}
                  color={active ? ACCENT : "var(--tx4b)"} />}
                label={item.label} count={pageCount(item.key)} active={active}
                badge={badge}
                onClick={() => onSelect(item.key)}
                pointerHandlers={pointerHandlers} {...dropSlot(item.key)} />
              {/* 「置顶」紧跟在会话之下:它是会话库的一个范围,不是另一个页面 */}
              {item.key === "library" && scopeCounts.pinned > 0 && (
                <NavRow indent icon={<PinIcon size={metrics.navIcon - 2} />} label={labels.pinned}
                  count={scopeCounts.pinned} active={scopeActive("pinned")}
                  onClick={() => onSelectScope({ kind: "pinned" })} />
              )}
            </div>
          );
        })}

        {scopeCounts.agents.length > 0 && (
          <>
            <NavSection label={labels.agents} open={sections.isOpen("agents")}
              onToggle={() => sections.toggle("agents")} />
            {sections.isOpen("agents") && scopeCounts.agents.map(agent => (
              <NavRow key={agent.tool} indent
                icon={<ToolIcon tool={agent.tool} size={metrics.navIcon} />}
                label={labels.toolNames[agent.tool] || agent.tool}
                count={agent.count}
                active={scopeActive("agent") && scope.value === agent.tool}
                onClick={() => onSelectScope({ kind: "agent", value: agent.tool })} />
            ))}
          </>
        )}
        {/* 「收藏」= Finder 侧栏的个人收藏:只放用户钉过的项目,全集在资源栏的
            文件夹树里。空着也照样显示分区,否则用户不知道有这么个地方 */}
        <NavSection label={labels.favorites} open={sections.isOpen("favorites")}
          onToggle={() => sections.toggle("favorites")} />
        {sections.isOpen("favorites") && (favoriteProjects.length > 0 ? (
          favoriteProjects.map(project => (
            <NavRow key={project.dir} indent icon={<FolderGlyph size={metrics.navIcon} />}
              label={project.repo} count={project.count} title={project.dir}
              active={scopeActive("project") && scope.value === project.dir}
              draggable
              dragging={dragDir === project.dir}
              dropBefore={dropAt?.dir === project.dir && dropAt.position === "before"}
              dropAfter={dropAt?.dir === project.dir && dropAt.position === "after"}
              onDragStart={event => {
                setDragDir(project.dir);
                event.dataTransfer.effectAllowed = "move";
                // Firefox 不设 data 就不触发 drop
                try { event.dataTransfer.setData("text/plain", project.dir); } catch { /* 忽略 */ }
              }}
              onDragOver={event => {
                if (!dragDir || dragDir === project.dir) return;
                event.preventDefault();
                const box = event.currentTarget.getBoundingClientRect();
                const after = event.clientY > box.top + box.height / 2;
                setDropAt({ dir: project.dir, position: after ? "after" : "before" });
              }}
              onDrop={event => {
                event.preventDefault();
                if (dragDir && dropAt) {
                  onReorderFavorite(dragDir, dropAt.dir, dropAt.position);
                }
                setDragDir(null);
                setDropAt(null);
              }}
              onDragEnd={() => { setDragDir(null); setDropAt(null); }}
              onClick={() => onSelectScope({ kind: "project", value: project.dir })} />
          ))
        ) : (
          <div style={{ padding: "2px 10px 4px 20px", fontSize: 11.5, lineHeight: 1.5,
            color: "var(--tx5)" }}>{labels.favoritesEmpty}</div>
        ))}
        {scopeCounts.tags.length > 0 && (
          <>
            <NavSection label={labels.tags} open={sections.isOpen("tags")}
              onToggle={() => sections.toggle("tags")} />
            {sections.isOpen("tags") && scopeCounts.tags.map(item => (
              <NavRow key={item.tag} indent icon={<TagGlyph size={metrics.navIcon} />} label={item.tag}
                count={item.count}
                active={scopeActive("tag") && scope.value === item.tag}
                onClick={() => onSelectScope({ kind: "tag", value: item.tag })} />
            ))}
          </>
        )}
      </div>

      <div style={{ flex: "none", display: "flex", flexDirection: "column",
        alignItems: "stretch", gap: 1, marginTop: 6, paddingTop: 6,
        borderTop: ".5px solid var(--line)" }}>
        <NavRow icon={scanning ? <Spinner size={metrics.navIcon} />
          : <RescanIcon size={metrics.navIcon} color="currentColor" />}
          label={scanning ? labels.scanning : labels.rescan}
          onClick={scanning ? undefined : onRescan} />
        <NavRow guide="rail-settings"
          icon={<RailGlyph name="settings" size={metrics.navIcon}
            color={settingsOpen ? ACCENT : "var(--tx4b)"} />}
          label={labels.settings} active={settingsOpen} badge={settingsBadge}
          onClick={onToggleSettings} />
      </div>
      </div>
    </div>
  );
}
