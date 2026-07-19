// 图标库:工具品牌图标 + 导航轨/通用小图标(均为内联 SVG,便于着色)
const TOOL_PATHS = {
  claude: "m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z",
  codex: "M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z",
  opencode: "M22 24H2V0h20zM17 4.8H7v14.4h10z",
};

// 工具图标:圆角方底 + 品牌形 + 可选状态点
export function ToolIcon({ tool, size = 26, dot = null }) {
  const inner = Math.round(size * 0.56);
  return (
    <span style={{ position: "relative", display: "inline-flex", alignItems: "center",
      justifyContent: "center", width: size, height: size, borderRadius: 8,
      background: "#EEF2F6", border: "1px solid #E1E7EC", flex: "none" }}>
      <svg viewBox="0 0 24 24" style={{ width: inner, height: inner, fill: "#3C4A5A", display: "block" }}>
        <path d={TOOL_PATHS[tool] || TOOL_PATHS.claude} />
      </svg>
      {dot && <span style={{ position: "absolute", right: -3, bottom: -3, width: 10, height: 10,
        borderRadius: "50%", background: dot, boxShadow: "0 0 0 2px #fff" }} />}
    </span>
  );
}

const svg = (vb, w, h, html, extra) => (
  <svg viewBox={vb} style={{ width: w, height: h, ...extra }}
    dangerouslySetInnerHTML={{ __html: html }} />
);

export const SearchIcon = () => svg("0 0 16 16", 13, 13,
  '<circle cx="7" cy="7" r="5" fill="none" stroke="#9AA3AD" stroke-width="1.5"/><line x1="10.6" y1="10.6" x2="14" y2="14" stroke="#9AA3AD" stroke-width="1.5" stroke-linecap="round"/>',
  { flex: "none" });

export const FilterIcon = () => svg("0 0 16 16", 12, 12,
  '<path d="M2 4h12M4.5 8h7M6.5 12h3" stroke="#6B7682" stroke-width="1.5" stroke-linecap="round"/>');

export const Caret = ({ open, size = 9 }) => svg("0 0 12 12", size, size,
  '<path d="M4 2l4 4-4 4" fill="none" stroke="#9AA3AD" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>',
  { flex: "none", transform: open ? "rotate(90deg)" : "rotate(0deg)", transition: "transform .16s ease" });

export const SortCaret = () => svg("0 0 16 16", 10, 10,
  '<path d="M4 6l4 4 4-4" fill="none" stroke="#9AA3AD" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>');

export const Spinner = ({ size = 13, accent = "#0B67F5", track = "#c7ced6" }) => svg("0 0 16 16", size, size,
  `<circle cx="8" cy="8" r="6" fill="none" stroke="${track}" stroke-width="2"/><path d="M8 2 a6 6 0 0 1 6 6" fill="none" stroke="${accent}" stroke-width="2" stroke-linecap="round"/>`,
  { animation: "fspin .8s linear infinite", flex: "none" });

export const RescanIcon = () => svg("0 0 16 16", 13, 13,
  '<path d="M13 8a5 5 0 1 1-1.5-3.5M13 3v2h-2" fill="none" stroke="#334155" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>');

export const SidebarIcon = () => svg("0 0 18 18", 16, 16,
  '<rect x="2" y="3.5" width="14" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><line x1="6.8" y1="3.5" x2="6.8" y2="14.5" stroke="currentColor" stroke-width="1.4"/><rect x="3.4" y="5.4" width="2" height="1.2" rx=".4" fill="currentColor"/><rect x="3.4" y="7.6" width="2" height="1.2" rx=".4" fill="currentColor"/>');

export const CheckBadge = ({ size = 18 }) => (
  <span style={{ width: size, height: size, borderRadius: "50%", background: "#1C9E5A",
    display: "inline-flex", alignItems: "center", justifyContent: "center", flex: "none" }}>
    {svg("0 0 12 12", size * 0.6, size * 0.6,
      '<path d="M2.5 6.2 5 8.6 9.5 3.6" fill="none" stroke="#fff" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>')}
  </span>
);

export const WarnTriangle = () => svg("0 0 16 16", 16, 16,
  '<path d="M8 1.5 15 14H1z" fill="none" stroke="#C4564C" stroke-width="1.3" stroke-linejoin="round"/><line x1="8" y1="6" x2="8" y2="9.5" stroke="#C4564C" stroke-width="1.3" stroke-linecap="round"/><circle cx="8" cy="11.6" r=".8" fill="#C4564C"/>',
  { flex: "none", marginTop: 1 });

export const PlusIcon = () => svg("0 0 16 16", 14, 14,
  '<line x1="8" y1="3" x2="8" y2="13" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><line x1="3" y1="8" x2="13" y2="8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>');

// 导航轨图标
const RAIL = {
  library: '<rect x="2" y="3.4" width="12" height="1.9" rx=".9" fill="currentColor"/><rect x="2" y="7.05" width="12" height="1.9" rx=".9" fill="currentColor"/><rect x="2" y="10.7" width="8" height="1.9" rx=".9" fill="currentColor"/>',
  history: '<path d="M3 5.4h7.4M8.1 3 10.6 5.4 8.1 7.8" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/><path d="M13 10.6H5.6M7.9 8.2 5.4 10.6 7.9 13" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
  snapshots: '<rect x="2.6" y="4.4" width="8" height="8" rx="1.6" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M5.6 4.4V3.3a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-1.1" fill="none" stroke="currentColor" stroke-width="1.5"/>',
  data: '<ellipse cx="8" cy="4.2" rx="4.8" ry="1.9" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M3.2 4.2v7.6c0 1 2.1 1.9 4.8 1.9s4.8-.9 4.8-1.9V4.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M3.2 8c0 1 2.1 1.9 4.8 1.9s4.8-.9 4.8-1.9" fill="none" stroke="currentColor" stroke-width="1.4"/>',
  guide: '<circle cx="8" cy="8" r="6.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M6.3 6.2a1.7 1.7 0 1 1 2.2 1.6c-.5.2-.7.5-.7 1" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><circle cx="8" cy="11" r=".75" fill="currentColor"/>',
  settings: '<circle cx="8" cy="8" r="2.1" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M8 1.7v1.5M8 12.8v1.5M14.3 8h-1.5M3.2 8H1.7M12.4 3.6l-1 1M4.6 11.4l-1 1M12.4 12.4l-1-1M4.6 4.6l-1-1" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>',
};

export const RailGlyph = ({ name, color = "#7A8591", size = 19 }) =>
  svg("0 0 16 16", size, size, RAIL[name], { color });
