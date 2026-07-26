import { AGENTS } from "../contracts/generated/agents.js";
import { PROVIDER_ICON } from "./providerIcons.js";
import { roleColorVar, roleIconPath } from "./roleIcons.js";

// 图标库:工具品牌图标 + 导航轨/通用小图标
//
// Agent 图标是资源而非代码:构建期从 assets/icons/<icon>.svg 静态收集,
// key 取自 contracts/agents.json 的 icon 字段。新增 Agent 只需放一个自
// 包含且带 viewBox 的 svg(自带底色与留白),不必修改本文件。
const AGENT_ICON = Object.fromEntries(
  Object.entries(
    import.meta.glob("../../assets/icons/*.svg", {
      eager: true, query: "?raw", import: "default",
    }),
  ).map(([path, markup]) => [path.split("/").pop().replace(/\.svg$/, ""), markup]),
);
const AGENT_FALLBACK_COLOR = {
  claude: "#D97757",
  codex: "#10A37F",
  opencode: "#4C7EDB",
  pi: "#9268C9",
  grok: "#6B7682",
};
// 工具图标:圆角方底 + 品牌形 + 可选状态点。
// 单色资源由渲染端补底色；未提供资源的 Agent 用首字母占位。
export function ToolIcon({ tool, size = 26, dot = null }) {
  const markup = AGENT_ICON[AGENTS[tool]?.icon || tool];
  const hasBakedBackground = markup?.includes("<rect");
  const fallbackColor = AGENT_FALLBACK_COLOR[tool] || "var(--tx3b)";
  return (
    <span className="noinvert" style={{ position: "relative", display: "inline-flex",
      alignItems: "center", justifyContent: "center", width: size, height: size, borderRadius: 8,
      background: hasBakedBackground
        ? undefined
        : `color-mix(in srgb, ${fallbackColor} 16%, var(--surface))`,
      color: hasBakedBackground ? undefined : fallbackColor,
      border: "1px solid var(--line)", overflow: "hidden", flex: "none" }}>
      {markup ? (
        <span className="agent-icon-svg"
          style={{ width: size, height: size, display: "block" }}
          dangerouslySetInnerHTML={{ __html: markup }} />
      ) : (
        <svg viewBox="0 0 24 24" style={{ width: size, height: size, display: "block" }}>
          <text x="12" y="16.5" textAnchor="middle" fontSize="13" fontWeight="700"
            fill="var(--tx3b)">{String(tool || "?")[0].toUpperCase()}</text>
        </svg>
      )}
      {dot && <span style={{ position: "absolute", right: -3, bottom: -3, width: 10, height: 10,
        borderRadius: "50%", background: dot, boxShadow: "0 0 0 2px var(--dot-ring)" }} />}
    </span>
  );
}

// 角色头像:图标库里的线条图标 + 角色配色,底色由配色现调,列表与详情共用一套尺寸比例
export function RoleAvatar({ icon, color, size = 28, radius }) {
  const tint = roleColorVar(color);
  return (
    <span style={{ width: size, height: size, borderRadius: radius ?? size * 0.32,
      flex: "none", display: "inline-flex", alignItems: "center", justifyContent: "center",
      background: `color-mix(in srgb, ${tint} 15%, transparent)`, color: tint }}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
        strokeLinecap="round" strokeLinejoin="round" aria-hidden
        style={{ width: Math.round(size * 0.56), height: Math.round(size * 0.56), display: "block" }}
        dangerouslySetInnerHTML={{ __html: roleIconPath(icon) }} />
    </span>
  );
}

// Provider 品牌图标:认识的用真实商标,不认识的用首字母,尺寸统一便于对齐
export function ProviderIcon({ provider, size = 16 }) {
  const icon = PROVIDER_ICON[provider];
  if (!icon) {
    return (
      <span style={{ width: size, height: size, borderRadius: 4, flex: "none",
        background: "var(--fill3)", color: "var(--tx3b)", display: "inline-flex",
        alignItems: "center", justifyContent: "center",
        fontSize: Math.round(size * 0.6), fontWeight: 700, lineHeight: 1 }}>
        {String(provider || "?")[0].toUpperCase()}</span>
    );
  }
  return (
    <svg viewBox={icon.viewBox} aria-hidden
      className={icon.mono ? undefined : "noinvert"}
      fill={icon.mono ? "currentColor" : undefined}
      style={{ width: size, height: size, flex: "none", display: "block" }}
      {...(icon.fillRule ? { fillRule: "evenodd" } : {})}
      dangerouslySetInnerHTML={{ __html: icon.body }} />
  );
}

const svg = (vb, w, h, html, extra) => (
  <svg viewBox={vb} style={{ width: w, height: h, ...extra }}
    dangerouslySetInnerHTML={{ __html: html }} />
);

export const SearchIcon = () => svg("0 0 16 16", 13, 13,
  '<circle cx="7" cy="7" r="5" fill="none" stroke="currentColor" stroke-width="1.5"/><line x1="10.6" y1="10.6" x2="14" y2="14" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>',
  { flex: "none", color: "var(--tx5)" });

export const FilterIcon = () => svg("0 0 16 16", 12, 12,
  '<path d="M2 4h12M4.5 8h7M6.5 12h3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>',
  { color: "var(--tx3b)" });

// 默认朝右;open 展开朝下,dir="left" 用于二级菜单的返回箭头
export const Caret = ({ open, size = 9, dir }) => svg("0 0 12 12", size, size,
  '<path d="M4 2l4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none", color: "var(--tx5)",
    transform: dir === "left" ? "rotate(180deg)" : open ? "rotate(90deg)" : "rotate(0deg)",
    transition: "transform .16s ease" });

export const SortCaret = () => svg("0 0 16 16", 10, 10,
  '<path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
  { color: "var(--tx5)" });

export const Spinner = ({ size = 13, accent = "var(--accent)", track = "var(--spin-track)" }) => svg("0 0 16 16", size, size,
  `<circle cx="8" cy="8" r="6" fill="none" style="stroke:${track}" stroke-width="2"/><path d="M8 2 a6 6 0 0 1 6 6" fill="none" style="stroke:${accent}" stroke-width="2" stroke-linecap="round"/>`,
  { animation: "fspin .8s linear infinite", flex: "none" });

export const RescanIcon = ({ size = 13, color = "var(--tx2)" } = {}) => svg("0 0 16 16", size, size,
  '<path d="M13 8a5 5 0 1 1-1.5-3.5M13 3v2h-2" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
  { color });

export const SidebarIcon = () => svg("0 0 18 18", 16, 16,
  '<rect x="2" y="3.5" width="14" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><line x1="6.8" y1="3.5" x2="6.8" y2="14.5" stroke="currentColor" stroke-width="1.4"/><rect x="3.4" y="5.4" width="2" height="1.2" rx=".4" fill="currentColor"/><rect x="3.4" y="7.6" width="2" height="1.2" rx=".4" fill="currentColor"/>');

export const CheckBadge = ({ size = 18 }) => (
  <span style={{ width: size, height: size, borderRadius: "50%", background: "var(--ok)",
    display: "inline-flex", alignItems: "center", justifyContent: "center", flex: "none" }}>
    {svg("0 0 12 12", size * 0.6, size * 0.6,
      '<path d="M2.5 6.2 5 8.6 9.5 3.6" fill="none" stroke="#fff" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>')}
  </span>
);

export const WarnTriangle = () => svg("0 0 16 16", 16, 16,
  '<path d="M8 1.5 15 14H1z" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/><line x1="8" y1="6" x2="8" y2="9.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><circle cx="8" cy="11.6" r=".8" fill="currentColor"/>',
  { flex: "none", marginTop: 1, color: "var(--err2)" });

// 齿轮(GitHub Octicons gear-16)
const GEAR_PATH = "M8 0a8.2 8.2 0 0 1 .701.031C9.444.095 9.99.645 10.16 1.29l.288 1.107c.018.066.079.158.212.224.231.114.454.243.668.386.123.082.233.09.299.071l1.103-.303c.644-.176 1.392.021 1.82.63.27.385.506.792.704 1.218.315.675.111 1.422-.364 1.891l-.814.806c-.049.048-.098.147-.088.294.016.257.016.515 0 .772-.01.147.038.246.088.294l.814.806c.475.469.679 1.216.364 1.891a7.977 7.977 0 0 1-.704 1.217c-.428.61-1.176.807-1.82.63l-1.102-.302c-.067-.019-.177-.011-.3.071a5.909 5.909 0 0 1-.668.386c-.133.066-.194.158-.211.224l-.29 1.106c-.168.646-.715 1.196-1.458 1.26a8.006 8.006 0 0 1-1.402 0c-.743-.064-1.289-.614-1.458-1.26l-.289-1.106c-.018-.066-.079-.158-.212-.224a5.738 5.738 0 0 1-.668-.386c-.123-.082-.233-.09-.299-.071l-1.103.303c-.644.176-1.392-.021-1.82-.63a8.12 8.12 0 0 1-.704-1.218c-.315-.675-.111-1.422.363-1.891l.815-.806c.05-.048.098-.147.088-.294a6.214 6.214 0 0 1 0-.772c.01-.147-.038-.246-.088-.294l-.815-.806C.635 6.045.431 5.298.746 4.623a7.92 7.92 0 0 1 .704-1.217c.428-.61 1.176-.807 1.82-.63l1.102.302c.067.019.177.011.3-.071.214-.143.437-.272.668-.386.133-.066.194-.158.211-.224l.29-1.106C6.009.645 6.556.095 7.299.03 7.53.01 7.764 0 8 0Zm-.571 1.525c-.036.003-.108.036-.137.146l-.289 1.105c-.147.561-.549.967-.998 1.189-.173.086-.34.183-.5.29-.417.278-.97.423-1.529.27l-1.103-.303c-.109-.03-.175.016-.195.045-.22.312-.412.644-.573.99-.014.031-.021.11.059.19l.815.806c.411.406.562.957.53 1.456a4.709 4.709 0 0 0 0 .582c.032.499-.119 1.05-.53 1.456l-.815.806c-.081.08-.073.159-.059.19.162.346.353.677.573.989.02.03.085.076.195.046l1.102-.303c.56-.153 1.113-.008 1.53.27.161.107.328.204.501.29.447.222.85.629.997 1.189l.289 1.105c.029.109.101.143.137.146a6.6 6.6 0 0 0 1.142 0c.036-.003.108-.036.137-.146l.289-1.105c.147-.561.549-.967.998-1.189.173-.086.34-.183.5-.29.417-.278.97-.423 1.529-.27l1.103.303c.109.029.175-.016.195-.045.22-.313.411-.644.573-.99.014-.031.021-.11-.059-.19l-.815-.806c-.411-.406-.562-.957-.53-1.456a4.709 4.709 0 0 0 0-.582c-.032-.499.119-1.05.53-1.456l.815-.806c.081-.08.073-.159.059-.19a6.464 6.464 0 0 0-.573-.989c-.02-.03-.085-.076-.195-.046l-1.102.303c-.56.153-1.113.008-1.53-.27a4.44 4.44 0 0 0-.501-.29c-.447-.222-.85-.629-.997-1.189l-.289-1.105c-.029-.11-.101-.143-.137-.146a6.6 6.6 0 0 0-1.142 0Z M11 8a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z M9.5 8a1.5 1.5 0 1 0-3.001.001A1.5 1.5 0 0 0 9.5 8Z";

const RAIL = {
  overview: '<rect x="2.2" y="2.2" width="5.2" height="5.2" rx="1.2" fill="currentColor"/><rect x="8.6" y="2.2" width="5.2" height="5.2" rx="1.2" fill="currentColor"/><rect x="2.2" y="8.6" width="5.2" height="5.2" rx="1.2" fill="currentColor"/><rect x="8.6" y="8.6" width="5.2" height="5.2" rx="1.2" fill="currentColor"/>',
  library: '<rect x="2" y="3.4" width="12" height="1.9" rx=".9" fill="currentColor"/><rect x="2" y="7.05" width="12" height="1.9" rx=".9" fill="currentColor"/><rect x="2" y="10.7" width="8" height="1.9" rx=".9" fill="currentColor"/>',
  history: '<path d="M3 5.4h7.4M8.1 3 10.6 5.4 8.1 7.8" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/><path d="M13 10.6H5.6M7.9 8.2 5.4 10.6 7.9 13" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
  askferry: '<path d="M8 1.4 9.7 6.3 14.6 8 9.7 9.7 8 14.6 6.3 9.7 1.4 8 6.3 6.3Z" fill="currentColor"/>',
  settings: `<path fill-rule="evenodd" clip-rule="evenodd" fill="currentColor" d="${GEAR_PATH}"/>`,
};

export const RailGlyph = ({ name, color = "var(--tx4b)", size = 19 }) =>
  svg("0 0 16 16", size, size, RAIL[name], { color });

const SETTINGS_GLYPH = {
  prefs: `<g transform="scale(1.125)"><path fill-rule="evenodd" clip-rule="evenodd" fill="currentColor" d="${GEAR_PATH}"/></g>`,
  sources: '<ellipse cx="9" cy="4.6" rx="5.2" ry="2.1" stroke="currentColor" stroke-width="1.4" fill="none"/><path d="M3.8 4.6v8.8c0 1.16 2.33 2.1 5.2 2.1s5.2-.94 5.2-2.1V4.6M3.8 9c0 1.16 2.33 2.1 5.2 2.1s5.2-.94 5.2-2.1" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linecap="round"/>',
  updates: '<path d="M9 3.1v8.2m0 0 3-3m-3 3-3-3M4 14.4h10" fill="none" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round"/>',
  models: '<path d="M9 2.2 15 5.6 9 9 3 5.6 9 2.2Z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="m3 9 6 3.4L15 9M3 12.4l6 3.4 6-3.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>',
  providers: '<rect x="4.6" y="4.6" width="8.8" height="8.8" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><rect x="7.3" y="7.3" width="3.4" height="3.4" rx="1" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M7 2.4v2.2M11 2.4v2.2M7 13.4v2.2M11 13.4v2.2M2.4 7h2.2M2.4 11h2.2M13.4 7h2.2M13.4 11h2.2" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>',
  skills: '<path d="M9 2.1 11.1 6.4l4.7.7-3.4 3.3.8 4.7L9 12.9l-4.2 2.2.8-4.7L2.2 7.1l4.7-.7L9 2.1Z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>',
  roles: '<circle cx="9" cy="6" r="3" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M3.8 15.2c.6-3 2.3-4.5 5.2-4.5s4.6 1.5 5.2 4.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>',
};

export const SetGlyph = ({ name, color = "var(--tx3b)" }) =>
  svg("0 0 18 18", 17, 17, SETTINGS_GLYPH[name], { color, flex: "none" });

export const CopyIcon = ({ size = 13 }) => svg("0 0 16 16", size, size,
  '<rect x="5.5" y="5.5" width="8.5" height="8.5" rx="1.8" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M3.2 10.5h-.4a1.3 1.3 0 0 1-1.3-1.3V3.3A1.3 1.3 0 0 1 2.8 2h5.9a1.3 1.3 0 0 1 1.3 1.3v.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>',
  { flex: "none" });

export const CheckIcon = ({ size = 13 }) => svg("0 0 16 16", size, size,
  '<path d="M3 8.5l3.4 3.4L13 5.2" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

// 手动模式:摊开的手掌。用 24 画布重画,墨迹上下都到边(2~22),重心正好落在画布中心;
// 旧的 16 画布版本墨迹压在下半部(5.3~14.8),居中的是画布不是墨迹,并排时会掉下去
export const ManualModeIcon = ({ size = 14 }) => svg("0 0 24 24", size, size,
  '<path d="M18 11V6a2 2 0 0 0-4 0M14 10V4a2 2 0 0 0-4 0v2M10 10.5V6a2 2 0 0 0-4 0v8M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.9-6-2.3l-3.6-3.6a2 2 0 0 1 2.8-2.8L7 15" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const AutoModeIcon = ({ size = 14 }) => svg("0 0 16 16", size, size,
  '<path d="M9.2 1.8 3.3 9h3.9l-.4 5.2L12.7 7H8.8l.4-5.2Z" fill="none" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const CloseIcon = ({ size = 12 }) => svg("0 0 16 16", size, size,
  '<path d="M4 4l8 8M12 4l-8 8" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>',
  { flex: "none" });

export const PencilIcon = ({ size = 13 }) => svg("0 0 16 16", size, size,
  '<path d="M3 13.2l.7-3 7.6-7.6a1.25 1.25 0 0 1 1.8 0l.5.5a1.25 1.25 0 0 1 0 1.8L6 12.5l-3 .7z" fill="none" stroke="currentColor" stroke-width="1.35" stroke-linejoin="round"/><path d="M10.2 3.7l1.9 1.9" stroke="currentColor" stroke-width="1.35"/>',
  { flex: "none" });

export const TrashIcon = ({ size = 13 }) => svg("0 0 16 16", size, size,
  '<path d="M2.5 4.2h11M6.4 4.2V3a.9.9 0 0 1 .9-.9h1.4a.9.9 0 0 1 .9.9v1.2M4.1 4.2l.6 8.7a1.2 1.2 0 0 0 1.2 1.1h4.2a1.2 1.2 0 0 0 1.2-1.1l.6-8.7" fill="none" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"/><path d="M6.7 7v4M9.3 7v4" stroke="currentColor" stroke-width="1.35" stroke-linecap="round"/>',
  { flex: "none" });

export const UndoIcon = ({ size = 13 }) => svg("0 0 16 16", size, size,
  '<path d="M3.5 6.5h6a3.6 3.6 0 1 1 0 7.2H6" fill="none" stroke="currentColor" stroke-width="1.45" stroke-linecap="round"/><path d="M6.3 3.7L3.5 6.5l2.8 2.8" fill="none" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const BookmarkIcon = ({ size = 12 }) => svg("0 0 16 16", size, size,
  '<path d="M4.2 2.5h7.6a.6.6 0 0 1 .6.6v10.4l-4.4-3-4.4 3V3.1a.6.6 0 0 1 .6-.6z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>',
  { flex: "none" });

export const ImageGlyph = ({ size = 12 }) => svg("0 0 16 16", size, size,
  '<rect x="2" y="3" width="12" height="10" rx="1.6" fill="none" stroke="currentColor" stroke-width="1.3"/><circle cx="5.6" cy="6.4" r="1.1" fill="currentColor"/><path d="M3.6 11.6l3-3 2 2 2.4-2.4 1.9 1.9" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const GearMini = ({ size = 11 }) => svg("0 0 16 16", size, size,
  `<path fill-rule="evenodd" clip-rule="evenodd" fill="currentColor" d="${GEAR_PATH}"/>`,
  { flex: "none" });

export const RefreshIcon = ({ size = 15 }) => svg("0 0 16 16", size, size,
  '<path d="M13.2 8a5.2 5.2 0 1 1-1.55-3.7M13.2 2.6v2.6h-2.6" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const TerminalIcon = ({ size = 15 }) => svg("0 0 16 16", size, size,
  '<rect x="1.6" y="2.6" width="12.8" height="10.8" rx="2.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M4.4 6l2.2 2-2.2 2M8 10.4h3.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const MigrateIcon = ({ size = 15 }) => svg("0 0 16 16", size, size,
  '<path d="M9.6 3.2h3a1.2 1.2 0 0 1 1.2 1.2v7.2a1.2 1.2 0 0 1-1.2 1.2h-3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><path d="M1.8 8h8M7 4.8 10.2 8 7 11.2" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const MoreDots = ({ size = 13 }) => svg("0 0 16 16", size, size,
  '<circle cx="3.4" cy="8" r="1.35" fill="currentColor"/><circle cx="8" cy="8" r="1.35" fill="currentColor"/><circle cx="12.6" cy="8" r="1.35" fill="currentColor"/>',
  { flex: "none" });

export const ArchiveIcon = ({ size = 12 }) => svg("0 0 16 16", size, size,
  '<rect x="1.8" y="2.6" width="12.4" height="3.4" rx="1" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M3 6v6a1.4 1.4 0 0 0 1.4 1.4h7.2A1.4 1.4 0 0 0 13 12V6M6.4 8.6h3.2" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>',
  { flex: "none" });

export const PinIcon = ({ size = 12, filled = false }) => svg("0 0 24 24", size, size,
  `<path d="M12 17v5M9 4h6l1 7 2 2H6l2-2 1-7z" fill="${filled ? "currentColor" : "none"}" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>`,
  { flex: "none" });

export const SendArrowIcon = ({ size = 14 }) => svg("0 0 16 16", size, size,
  '<path d="M8 13V3.4M3.8 7.2 8 3l4.2 4.2" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const StopFillIcon = ({ size = 11 }) => svg("0 0 16 16", size, size,
  '<rect x="3.2" y="3.2" width="9.6" height="9.6" rx="2.4" fill="currentColor"/>',
  { flex: "none" });

export const PlusIcon = ({ size = 13 }) => svg("0 0 16 16", size, size,
  '<path d="M8 2.8v10.4M2.8 8h10.4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>',
  { flex: "none" });
