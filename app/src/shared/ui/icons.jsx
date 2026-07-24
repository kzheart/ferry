import { PROVIDER_ICON } from "./providerIcons.js";

// 图标库:工具品牌图标 + 导航轨/通用小图标(均为内联 SVG,便于着色)
// 三个工具的图形均取自各自官方来源,保留品牌原色:
//   claude   — Claude Code 终端图标(Claude 橙放射星,来自 claude.ai/favicon.svg)
//   codex    — Codex CLI 背后的 OpenAI 六瓣花(来自 OpenAI 官方 logo / opencode 仓库 provider 图标)
//   opencode — OpenCode 桌面 app 图标 v3(深色方底 + 白色相框 + 灰色内方块,来自 opencode 仓库 favicon-v3.svg)
const CLAUDE_PATH = "M52.4285 162.873L98.7844 136.879L99.5485 134.602L98.7844 133.334H96.4921L88.7237 132.862L62.2346 132.153L39.3113 131.207L17.0249 130.026L11.4214 128.844L6.2 121.873L6.7094 118.447L11.4214 115.257L18.171 115.847L33.0711 116.911L55.485 118.447L71.6586 119.392L95.728 121.873H99.5485L100.058 120.337L98.7844 119.392L97.7656 118.447L74.5877 102.732L49.4995 86.1905L36.3823 76.62L29.3779 71.7757L25.8121 67.2858L24.2839 57.3608L30.6515 50.2716L39.3113 50.8623L41.4763 51.4531L50.2636 58.1879L68.9842 72.7209L93.4357 90.6804L97.0015 93.6343L98.4374 92.6652L98.6571 91.9801L97.0015 89.2625L83.757 65.2772L69.621 40.8192L63.2534 30.6579L61.5978 24.632C60.9565 22.1032 60.579 20.0111 60.579 17.4246L67.8381 7.49965L71.9133 6.19995L81.7193 7.49965L85.7946 11.0443L91.9074 24.9865L101.714 46.8451L116.996 76.62L121.453 85.4816L123.873 93.6343L124.764 96.1155H126.292V94.6976L127.566 77.9197L129.858 57.3608L132.15 30.8942L132.915 23.4505L136.608 14.4708L143.994 9.62643L149.725 12.344L154.437 19.0788L153.8 23.4505L150.998 41.6463L145.522 70.1215L141.957 89.2625H143.994L146.414 86.7813L156.093 74.0206L172.266 53.698L179.398 45.6635L187.803 36.802L193.152 32.5484H203.34L210.726 43.6549L207.415 55.1159L196.972 68.3492L188.312 79.5739L175.896 96.2095L168.191 109.585L168.882 110.689L170.738 110.53L198.755 104.504L213.91 101.787L231.994 98.7149L240.144 102.496L241.036 106.395L237.852 114.311L218.495 119.037L195.826 123.645L162.07 131.592L161.696 131.893L162.137 132.547L177.36 133.925L183.855 134.279H199.774L229.447 136.524L237.215 141.605L241.8 147.867L241.036 152.711L229.065 158.737L213.019 154.956L175.45 145.977L162.587 142.787H160.805V143.85L171.502 154.366L191.242 172.089L215.82 195.011L217.094 200.682L213.91 205.172L210.599 204.699L188.949 188.394L180.544 181.069L161.696 165.118H160.422V166.772L164.752 173.152L187.803 207.771L188.949 218.405L187.294 221.832L181.308 223.959L174.813 222.777L161.187 203.754L147.305 182.486L136.098 163.345L134.745 164.2L128.075 235.42L125.019 239.082L117.887 241.8L111.902 237.31L108.718 229.984L111.902 215.452L115.722 196.547L118.779 181.541L121.58 162.873L123.291 156.636L123.14 156.219L121.773 156.449L107.699 175.752L86.304 204.699L69.3663 222.777L65.291 224.431L58.2867 220.768L58.9235 214.27L62.8713 208.48L86.304 178.705L100.44 160.155L109.551 149.507L109.462 147.967L108.959 147.924L46.6977 188.512L35.6182 189.93L30.7788 185.44L31.4156 178.115L33.7079 175.752L52.4285 162.873Z";
const CODEX_PATH = "M32.8377 17.282C33.2127 16.25 33.3072 15.218 33.2127 14.1875C33.1197 13.1571 32.7447 12.1251 32.2752 11.1876C31.4322 9.78209 30.2127 8.6571 28.8072 8.0001C27.3072 7.34461 25.7127 7.15711 24.1197 7.53211C23.3698 6.78212 22.5253 6.12512 21.5878 5.65713C20.6503 5.18913 19.5253 5.00013 18.4948 5.00013C16.8851 4.99074 15.3125 5.48246 13.9948 6.40712C12.6824 7.34311 11.7449 8.6571 11.2754 10.1571C10.1504 10.4376 9.21289 10.9071 8.27539 11.4696C7.4324 12.1251 6.77541 12.9696 6.21291 13.8126C5.36992 15.2195 5.08792 16.8125 5.27542 18.407C5.46399 19.9968 6.11605 21.496 7.1504 22.718C6.79608 23.7086 6.66795 24.7659 6.77541 25.8124C6.86991 26.8444 7.2449 27.8749 7.7129 28.8124C8.55739 30.2194 9.77538 31.3444 11.1824 31.9999C12.6824 32.6569 14.2753 32.8444 15.8698 32.4694C16.6198 33.2194 17.4628 33.8749 18.4003 34.3444C19.3378 34.8139 20.4628 34.9999 21.4948 34.9999C23.1043 35.0097 24.6769 34.5185 25.9947 33.5944C27.3072 32.6569 28.2447 31.3444 28.7127 29.8444C29.7719 29.6432 30.7682 29.1934 31.6197 28.5319C32.4627 27.8749 33.2127 27.1249 33.6822 26.1874C34.5251 24.7819 34.8071 23.1875 34.6196 21.5945C34.4322 20 33.8697 18.5015 32.8377 17.282ZM21.5878 33.0304C20.0878 33.0304 18.9628 32.5609 17.9323 31.7179C17.9323 31.7179 18.0253 31.6234 18.1198 31.6234L24.1197 28.1554C24.2862 28.0803 24.4196 27.9469 24.4947 27.7804C24.5698 27.636 24.6021 27.4731 24.5877 27.3109V18.875L27.1197 20.375V27.3124C27.1455 28.0547 27.0215 28.7945 26.755 29.4878C26.4885 30.181 26.085 30.8134 25.5687 31.3473C25.0523 31.8811 24.4337 32.3054 23.7497 32.5949C23.0658 32.8843 22.3305 33.0314 21.5878 33.0304ZM9.49488 27.8749C8.83789 26.7499 8.55739 25.4374 8.83789 24.125C8.83789 24.125 8.93239 24.2195 9.02539 24.2195L15.0253 27.6874C15.1693 27.7638 15.3325 27.7966 15.4948 27.7819C15.6823 27.7819 15.8698 27.7819 15.9628 27.6874L23.2753 23.4695V26.3749L17.1823 29.9374C16.5506 30.3042 15.8527 30.5427 15.1287 30.6393C14.4046 30.7358 13.6686 30.6884 12.9629 30.4999C11.4629 30.1249 10.2449 29.1874 9.49488 27.8749ZM7.9004 14.8445C8.56239 13.7234 9.58826 12.8627 10.8074 12.4056V19.532C10.8074 19.718 10.8074 19.907 10.9004 20C10.9755 20.1665 11.1089 20.2998 11.2754 20.375L18.5878 24.5944L16.0573 26.0944L10.0574 22.625C9.41842 22.2639 8.85742 21.7797 8.40684 21.2004C7.95627 20.6211 7.62506 19.9582 7.4324 19.25C7.05741 17.8445 7.1504 16.157 7.9004 14.8445ZM28.6197 19.625L21.3073 15.407L23.8377 13.9071L29.8377 17.375C30.7752 17.9375 31.5252 18.6875 31.9947 19.625C32.4642 20.5625 32.7447 21.5945 32.6502 22.7195C32.5603 23.7755 32.1699 24.7837 31.5252 25.6249C30.8697 26.4694 30.0252 27.1249 28.9947 27.4999V20.375C28.9947 20.1875 28.9947 20 28.9002 19.907C28.9002 19.907 28.8072 19.718 28.6197 19.625ZM31.1502 15.875C31.1502 15.875 31.0572 15.782 30.9627 15.782L24.9627 12.3126C24.7752 12.2196 24.6822 12.2196 24.4947 12.2196C24.3072 12.2196 24.1197 12.2196 24.0252 12.3126L16.7128 16.532V13.6251L22.8073 10.0626C23.7448 9.50009 24.7752 9.31259 25.9002 9.31259C26.9322 9.31259 27.9627 9.68759 28.9002 10.3446C29.7447 11.0001 30.4947 11.8446 30.8697 12.7821C31.2447 13.7196 31.3377 14.8445 31.1502 15.875ZM15.4003 21.125L12.8699 19.625V12.5946C12.8699 11.5626 13.1503 10.4376 13.7128 9.59459C14.2753 8.6571 15.1198 8.0001 16.0573 7.53211C17.0127 7.05249 18.0956 6.88812 19.1503 7.06261C20.1823 7.15711 21.2128 7.62511 22.0573 8.2821C22.0573 8.2821 21.9628 8.3751 21.8698 8.3751L15.8698 11.8446C15.7033 11.9197 15.57 12.0531 15.4948 12.2196C15.4003 12.4071 15.4003 12.5001 15.4003 12.6876V21.125ZM16.7128 18.125L19.9948 16.25L23.2753 18.125V21.875L19.9948 23.75L16.7128 21.875V18.125Z";
const OPENCODE_FRAME_PATH = "M384 416H128V96H384V416ZM320 160H192V352H320V160Z";
const OPENCODE_BLOCK_PATH = "M320 224V352H192V224H320Z";

const TOOL_ICON = {
  claude: {
    viewBox: "0 0 248 248",
    bg: "#FFFFFF",
    innerRatio: 0.66,
    children: <path d={CLAUDE_PATH} fill="#D97757" />,
  },
  codex: {
    viewBox: "0 0 40 40",
    bg: "#FFFFFF",
    innerRatio: 0.66,
    children: <path d={CODEX_PATH} fill="#0F0F0F" />,
  },
  opencode: {
    viewBox: "0 0 512 512",
    bg: "#131010",
    innerRatio: 1.0,
    children: (
      <>
        <path d={OPENCODE_FRAME_PATH} fill="#FFFFFF" fillRule="evenodd" clipRule="evenodd" />
        <path d={OPENCODE_BLOCK_PATH} fill="#5A5858" />
      </>
    ),
  },
};

// 未识别的 icon id 用首字母占位,保证新增 Agent 不改前端也有图标
const fallbackIcon = tool => ({
  viewBox: "0 0 24 24",
  bg: "var(--fill3)",
  innerRatio: 1.0,
  children: (
    <text x="12" y="16.5" textAnchor="middle" fontSize="13" fontWeight="700"
      fill="var(--tx3b)">{String(tool || "?")[0].toUpperCase()}</text>
  ),
});

// 工具图标:圆角方底 + 品牌形 + 可选状态点
export function ToolIcon({ tool, size = 26, dot = null }) {
  const icon = TOOL_ICON[tool] || fallbackIcon(tool);
  const inner = Math.round(size * icon.innerRatio);
  return (
    <span className="noinvert" style={{ position: "relative", display: "inline-flex",
      alignItems: "center", justifyContent: "center", width: size, height: size, borderRadius: 8,
      background: icon.bg, border: "1px solid var(--line)", overflow: "hidden", flex: "none" }}>
      <svg viewBox={icon.viewBox} style={{ width: inner, height: inner, display: "block" }}>
        {icon.children}
      </svg>
      {dot && <span style={{ position: "absolute", right: -3, bottom: -3, width: 10, height: 10,
        borderRadius: "50%", background: dot, boxShadow: "0 0 0 2px var(--dot-ring)" }} />}
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

// 导航轨图标
const RAIL = {
  overview: '<rect x="2.2" y="2.2" width="5.2" height="5.2" rx="1.2" fill="currentColor"/><rect x="8.6" y="2.2" width="5.2" height="5.2" rx="1.2" fill="currentColor"/><rect x="2.2" y="8.6" width="5.2" height="5.2" rx="1.2" fill="currentColor"/><rect x="8.6" y="8.6" width="5.2" height="5.2" rx="1.2" fill="currentColor"/>',
  library: '<rect x="2" y="3.4" width="12" height="1.9" rx=".9" fill="currentColor"/><rect x="2" y="7.05" width="12" height="1.9" rx=".9" fill="currentColor"/><rect x="2" y="10.7" width="8" height="1.9" rx=".9" fill="currentColor"/>',
  history: '<path d="M3 5.4h7.4M8.1 3 10.6 5.4 8.1 7.8" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/><path d="M13 10.6H5.6M7.9 8.2 5.4 10.6 7.9 13" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
  askferry: '<path d="M8 1.4 9.7 6.3 14.6 8 9.7 9.7 8 14.6 6.3 9.7 1.4 8 6.3 6.3Z" fill="currentColor"/>',
  settings: `<path fill-rule="evenodd" clip-rule="evenodd" fill="currentColor" d="${GEAR_PATH}"/>`,
};

export const RailGlyph = ({ name, color = "var(--tx4b)", size = 19 }) =>
  svg("0 0 16 16", size, size, RAIL[name], { color });

// 设置页分类图标
const SETTINGS_GLYPH = {
  prefs: `<g transform="scale(1.125)"><path fill-rule="evenodd" clip-rule="evenodd" fill="currentColor" d="${GEAR_PATH}"/></g>`,
  sources: '<ellipse cx="9" cy="4.6" rx="5.2" ry="2.1" stroke="currentColor" stroke-width="1.4" fill="none"/><path d="M3.8 4.6v8.8c0 1.16 2.33 2.1 5.2 2.1s5.2-.94 5.2-2.1V4.6M3.8 9c0 1.16 2.33 2.1 5.2 2.1s5.2-.94 5.2-2.1" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linecap="round"/>',
  updates: '<path d="M9 3.1v8.2m0 0 3-3m-3 3-3-3M4 14.4h10" fill="none" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round"/>',
  models: '<path d="M9 2.2 15 5.6 9 9 3 5.6 9 2.2Z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="m3 9 6 3.4L15 9M3 12.4l6 3.4 6-3.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>',
  providers: '<rect x="4.6" y="4.6" width="8.8" height="8.8" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><rect x="7.3" y="7.3" width="3.4" height="3.4" rx="1" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M7 2.4v2.2M11 2.4v2.2M7 13.4v2.2M11 13.4v2.2M2.4 7h2.2M2.4 11h2.2M13.4 7h2.2M13.4 11h2.2" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>',
  roles: '<circle cx="9" cy="6" r="3" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M3.8 15.2c.6-3 2.3-4.5 5.2-4.5s4.6 1.5 5.2 4.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>',
};

export const SetGlyph = ({ name, color = "var(--tx3b)" }) =>
  svg("0 0 18 18", 17, 17, SETTINGS_GLYPH[name], { color, flex: "none" });

// 会话时间线操作图标(线条风,随 currentColor 着色)
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

// 详情页工具栏图标(16px 线条风,随 currentColor 着色)
export const RefreshIcon = ({ size = 15 }) => svg("0 0 16 16", size, size,
  '<path d="M13.2 8a5.2 5.2 0 1 1-1.55-3.7M13.2 2.6v2.6h-2.6" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const TerminalIcon = ({ size = 15 }) => svg("0 0 16 16", size, size,
  '<rect x="1.6" y="2.6" width="12.8" height="10.8" rx="2.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M4.4 6l2.2 2-2.2 2M8 10.4h3.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const MigrateIcon = ({ size = 15 }) => svg("0 0 16 16", size, size,
  '<path d="M9.6 3.2h3a1.2 1.2 0 0 1 1.2 1.2v7.2a1.2 1.2 0 0 1-1.2 1.2h-3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><path d="M1.8 8h8M7 4.8 10.2 8 7 11.2" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

// 侧栏行悬浮操作图标(11-12px)
export const MoreDots = ({ size = 13 }) => svg("0 0 16 16", size, size,
  '<circle cx="3.4" cy="8" r="1.35" fill="currentColor"/><circle cx="8" cy="8" r="1.35" fill="currentColor"/><circle cx="12.6" cy="8" r="1.35" fill="currentColor"/>',
  { flex: "none" });

export const ArchiveIcon = ({ size = 12 }) => svg("0 0 16 16", size, size,
  '<rect x="1.8" y="2.6" width="12.4" height="3.4" rx="1" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M3 6v6a1.4 1.4 0 0 0 1.4 1.4h7.2A1.4 1.4 0 0 0 13 12V6M6.4 8.6h3.2" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>',
  { flex: "none" });

export const PinIcon = ({ size = 12, filled = false }) => svg("0 0 24 24", size, size,
  `<path d="M12 17v5M9 4h6l1 7 2 2H6l2-2 1-7z" fill="${filled ? "currentColor" : "none"}" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>`,
  { flex: "none" });

// Ask Ferry 对话视图图标
export const SendArrowIcon = ({ size = 14 }) => svg("0 0 16 16", size, size,
  '<path d="M8 13V3.4M3.8 7.2 8 3l4.2 4.2" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none" });

export const StopFillIcon = ({ size = 11 }) => svg("0 0 16 16", size, size,
  '<rect x="3.2" y="3.2" width="9.6" height="9.6" rx="2.4" fill="currentColor"/>',
  { flex: "none" });

export const PlusIcon = ({ size = 13 }) => svg("0 0 16 16", size, size,
  '<path d="M8 2.8v10.4M2.8 8h10.4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>',
  { flex: "none" });
