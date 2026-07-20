// 首次启动:检测到的工具 + 开始扫描
import { TOOL_NAME, TOOLS } from "../../api/contract/tools.js";
import { ToolIcon } from "../../components/ui/icons.jsx";
import appIcon from "../../assets/app-icon.png";

export default function FirstRun({ env, scan, onStart }) {
  return (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
      minHeight: 0, background: "var(--inset)" }}>
      <div style={{ width: 460, background: "var(--surface)", border: "1px solid var(--line3)", borderRadius: 14,
        boxShadow: "0 12px 34px -18px rgba(20,28,38,.28)", padding: "30px 30px 26px",
        animation: "ffade .3s ease" }}>
        <img className="noinvert" src={appIcon} alt="Ferry" width={44} height={44} style={{ display: "block" }} />
        <div style={{ fontSize: 20, fontWeight: 650, marginTop: 16, letterSpacing: "-.01em" }}>
          欢迎使用 Ferry</div>
        <div style={{ fontSize: 13, color: "var(--tx3b)", marginTop: 6, lineHeight: 1.55 }}>
          Ferry 在本机运行,扫描你的 AI 编码工具会话并统一浏览。无需登录,不上传任何数据。</div>
        <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx5)", letterSpacing: ".04em",
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
              border: "1px solid var(--line5)", borderRadius: 9, marginBottom: 8 }}>
              <ToolIcon tool={t} size={26} dot={installed ? "var(--ok)" : "var(--line-strong)"} />
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, color: "var(--tx2)", fontWeight: 600 }}>{TOOL_NAME[t]}</div>
                <div style={{ fontSize: 11.5, color: "var(--tx4)" }}>{detect}</div>
              </div>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 11.5,
                color: installed ? "var(--ok-deep)" : "var(--tx5)", fontWeight: 600 }}>
                <span style={{ width: 7, height: 7, borderRadius: "50%",
                  background: installed ? "var(--ok)" : "var(--line-strong)" }} />
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
