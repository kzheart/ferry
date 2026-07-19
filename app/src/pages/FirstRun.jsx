// 首次启动:检测到的工具 + 开始扫描
import { ACCENT, TOOL_NAME, TOOLS } from "../api.js";
import { ToolIcon } from "../icons.jsx";

export default function FirstRun({ env, scan, onStart }) {
  return (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
      minHeight: 0, background: "#F7F9FB" }}>
      <div style={{ width: 460, background: "#fff", border: "1px solid #E4E9EE", borderRadius: 14,
        boxShadow: "0 12px 34px -18px rgba(20,28,38,.28)", padding: "30px 30px 26px",
        animation: "ffade .3s ease" }}>
        <div style={{ width: 44, height: 44, borderRadius: 11, background: ACCENT, display: "flex",
          alignItems: "center", justifyContent: "center", color: "#fff", fontWeight: 700,
          fontSize: 20 }}>F</div>
        <div style={{ fontSize: 20, fontWeight: 650, marginTop: 16, letterSpacing: "-.01em" }}>
          欢迎使用 Ferry</div>
        <div style={{ fontSize: 13, color: "#6B7682", marginTop: 6, lineHeight: 1.55 }}>
          Ferry 在本机运行,扫描你的 AI 编码工具会话并统一浏览。无需登录,不上传任何数据。</div>
        <div style={{ fontSize: 11, fontWeight: 600, color: "#9AA3AD", letterSpacing: ".04em",
          margin: "20px 0 8px" }}>检测到的工具</div>
        {TOOLS.map(t => {
          const info = env?.[t] || {};
          const tool = scan?.tools?.[t];
          const installed = info.installed;
          const detect = tool?.ok
            ? `${tool.path} · ${tool.count} 会话`
            : installed ? `v${info.version || "?"}` : "未检测到安装";
          return (
            <div key={t} style={{ display: "flex", alignItems: "center", gap: 11, padding: "10px 12px",
              border: "1px solid #E8ECF0", borderRadius: 9, marginBottom: 8 }}>
              <ToolIcon tool={t} size={26} dot={installed ? "#1C9E5A" : "#C3CBD3"} />
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, color: "#334155", fontWeight: 600 }}>{TOOL_NAME[t]}</div>
                <div style={{ fontSize: 11.5, color: "#8A939D" }}>{detect}</div>
              </div>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 11.5,
                color: installed ? "#1C7C43" : "#9AA3AD", fontWeight: 600 }}>
                <span style={{ width: 7, height: 7, borderRadius: "50%",
                  background: installed ? "#1C9E5A" : "#C3CBD3" }} />
                {installed ? "已检测" : "未安装"}
              </span>
            </div>
          );
        })}
        <button className="fbtn-primary" onClick={onStart}
          style={{ width: "100%", height: 40, marginTop: 12, borderRadius: 9, fontSize: 14 }}>
          开始扫描本机会话</button>
      </div>
    </div>
  );
}
