// 设置全屏页(按原型):左侧分类栏 + 常规 / 数据来源 / 外观 / 字体 / 关于
import { useState } from "react";
import { TOOL_NAME, TOOLS } from "../api.js";
import { ACCENTS, FONT_SIZES } from "../settings.js";
import { SetGlyph, ToolIcon } from "../icons.jsx";
import appIcon from "../assets/app-icon.png";

const VERSION = "1.4.0";

const SECTIONS = [
  ["general", "常规"], ["sources", "数据来源"],
  ["appearance", "外观"], ["type", "字体"], ["about", "关于"],
];

// ---------- 通用排版件 ----------
const GroupTitle = ({ children, first }) => (
  <div style={{ fontSize: 11.5, fontWeight: 700, color: "#9AA3AD", letterSpacing: ".05em",
    margin: first ? "0 0 9px 2px" : "22px 0 9px 2px" }}>{children}</div>
);

const Card = ({ children }) => (
  <div style={{ border: "1px solid #E7ECF0", borderRadius: 12, background: "#fff",
    overflow: "hidden" }}>{children}</div>
);

function Row({ title, desc, children, first }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 12, padding: "14px 16px",
      borderTop: first ? "none" : "1px solid #F0F3F6" }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: "#1C2530" }}>{title}</div>
        {desc && <div style={{ fontSize: 11.5, color: "#8A939D", marginTop: 2 }}>{desc}</div>}
      </div>
      {children}
    </div>
  );
}

// 分段按钮组
function Segmented({ options, value, onChange }) {
  return (
    <div style={{ display: "flex", gap: 6, flex: "none" }}>
      {options.map(([k, label]) => {
        const on = value === k;
        return (
          <button key={k} onClick={() => onChange(k)}
            style={{ height: 30, minWidth: 44, padding: "0 13px", borderRadius: 8,
              border: `1px solid ${on ? "var(--accent)" : "#DCE2E8"}`,
              background: on ? "var(--accent)" : "#fff", color: on ? "#fff" : "#334155",
              fontSize: 12.5, cursor: "pointer", fontWeight: 500 }}>{label}</button>
        );
      })}
    </div>
  );
}

function Toggle({ on, onChange }) {
  return (
    <button onClick={() => onChange(!on)} aria-pressed={on}
      style={{ width: 44, height: 26, borderRadius: 20, border: "none", flex: "none",
        background: on ? "var(--accent)" : "#CBD3DB", cursor: "pointer", padding: 0,
        position: "relative", transition: "background .15s ease" }}>
      <span style={{ position: "absolute", top: 3, left: on ? 21 : 3, width: 20, height: 20,
        borderRadius: "50%", background: "#fff", boxShadow: "0 1px 3px rgba(0,0,0,.28)",
        transition: "left .15s ease" }} />
    </button>
  );
}

// ---------- 各分区 ----------
function General({ guideSeen, onOpenGuide, onFirstRun }) {
  return (
    <div style={{ animation: "fslide .16s ease" }}>
      <GroupTitle first>应用</GroupTitle>
      <Card>
        <Row first title="版本">
          <div className="mono" style={{ fontSize: 12.5, color: "#6B7682" }}>{VERSION} · 本地版</div>
        </Row>
        <Row title="运行方式" desc="引擎为本机 Python 进程,不连任何服务器">
          <div style={{ fontSize: 12.5, color: "#6B7682" }}>本机</div>
        </Row>
        <Row title="界面语言">
          <div style={{ fontSize: 12.5, color: "#334155" }}>简体中文</div>
        </Row>
      </Card>

      <GroupTitle>上手引导</GroupTitle>
      <Card>
        <Row first title="功能引导" desc="重新观看导航、资源栏与迁移交付的分步讲解">
          <button className="fbtn-primary" style={{ height: 30, padding: "0 13px" }}
            onClick={onOpenGuide}>{guideSeen ? "重新查看引导" : "快速上手"}</button>
        </Row>
        <Row title="首次启动检测" desc="重新查看本机各工具的安装与会话检测结果">
          <button className="fbtn" style={{ height: 30, fontSize: 12.5 }}
            onClick={onFirstRun}>打开</button>
        </Row>
      </Card>
    </div>
  );
}

function Sources({ scan, env, scanning, onRescan }) {
  const tools = scan?.tools || {};
  const connected = TOOLS.filter(t => tools[t]?.ok).length;
  const total = TOOLS.reduce((a, t) => a + (tools[t]?.count || 0), 0);
  return (
    <div style={{ animation: "fslide .16s ease" }}>
      <div style={{ display: "flex", alignItems: "flex-end", margin: "0 0 9px 2px" }}>
        <div style={{ flex: 1, fontSize: 11.5, fontWeight: 700, color: "#9AA3AD",
          letterSpacing: ".05em" }}>已连接的工具</div>
        <div style={{ fontSize: 11.5, color: "#8A939D" }}>
          {connected} 个已连接 · {total} 个会话</div>
      </div>
      <Card>
        {TOOLS.map((t, i) => {
          const info = tools[t] || {};
          const ok = info.ok;
          return (
            <div key={t} style={{ display: "flex", alignItems: "center", gap: 13,
              padding: "14px 16px", borderTop: i === 0 ? "none" : "1px solid #F0F3F6" }}>
              <ToolIcon tool={t} size={30} dot={ok ? "#1C9E5A" : "#D5544A"} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: "#1C2530" }}>
                  {TOOL_NAME[t]}
                  {env?.[t]?.version && <span style={{ fontWeight: 400, color: "#9AA3AD",
                    fontSize: 11.5 }}> · v{env[t].version}</span>}
                </div>
                <div className="mono" style={{ fontSize: 11, color: "#9AA3AD", marginTop: 2,
                  whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                  {info.path || "—"}</div>
              </div>
              <div style={{ textAlign: "right", flex: "none", marginRight: 4 }}>
                <div style={{ fontSize: 12, color: "#6B7682" }}>
                  {ok ? `${info.count} 个会话` : (info.error || "不可用")}</div>
                <div style={{ display: "inline-flex", alignItems: "center", gap: 5, fontSize: 11.5,
                  fontWeight: 600, color: ok ? "#1C7C43" : "#B4433A", marginTop: 2 }}>
                  <span style={{ width: 6, height: 6, borderRadius: "50%",
                    background: ok ? "#1C9E5A" : "#D5544A" }} />{ok ? "已连接" : "扫描失败"}</div>
              </div>
              <button className="fbtn" style={{ height: 30, fontSize: 12, flex: "none" }}
                onClick={onRescan} disabled={scanning}>
                {scanning ? "扫描中…" : "重新扫描"}</button>
            </div>
          );
        })}
      </Card>
      <div style={{ fontSize: 11, color: "#9AA3AD", marginTop: 10, lineHeight: 1.55,
        paddingLeft: 2 }}>
        Ferry 在本机自动发现受支持工具的会话目录。源会话保持只读,不会被修改。</div>
    </div>
  );
}

function Appearance({ s, set }) {
  const themes = [
    ["light", "亮色", "#FBFCFD"],
    ["dark", "暗色", "#1C2530"],
    ["system", "跟随系统", "linear-gradient(105deg,#FBFCFD 0 50%,#1C2530 50% 100%)"],
  ];
  return (
    <div style={{ animation: "fslide .16s ease" }}>
      <GroupTitle first>主题</GroupTitle>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 12 }}>
        {themes.map(([k, label, sw]) => {
          const on = s.theme === k;
          return (
            <div key={k} onClick={() => set({ theme: k })}
              style={{ border: `1.5px solid ${on ? "var(--accent)" : "#E7ECF0"}`,
                background: on ? "#F3F7FE" : "#fff", borderRadius: 12, padding: 12,
                cursor: "pointer" }}>
              <div style={{ height: 54, borderRadius: 8, background: sw,
                border: "1px solid rgba(20,28,38,.10)" }} />
              <div style={{ display: "flex", alignItems: "center", gap: 7, marginTop: 11 }}>
                <span style={{ width: 15, height: 15, borderRadius: "50%", flex: "none",
                  border: `2px solid ${on ? "var(--accent)" : "#CBD3DB"}`, display: "inline-flex",
                  alignItems: "center", justifyContent: "center" }}>
                  <span style={{ width: 7, height: 7, borderRadius: "50%",
                    background: on ? "var(--accent)" : "transparent" }} /></span>
                <span style={{ fontSize: 12.5, fontWeight: 600, color: "#334155" }}>{label}</span>
              </div>
            </div>
          );
        })}
      </div>

      <GroupTitle>强调色</GroupTitle>
      <div style={{ border: "1px solid #E7ECF0", borderRadius: 12, background: "#fff",
        padding: "15px 16px", display: "flex", alignItems: "center", gap: 12 }}>
        {ACCENTS.map(c => {
          const on = s.accent.toLowerCase() === c.toLowerCase();
          return (
            <button key={c} onClick={() => set({ accent: c })} title={c}
              style={{ width: 30, height: 30, borderRadius: "50%", background: c, padding: 0,
                border: `2px solid ${on ? "#1C2530" : "transparent"}`, cursor: "pointer",
                display: "inline-flex", alignItems: "center", justifyContent: "center",
                color: "#fff", fontSize: 13, lineHeight: 1 }}>{on ? "✓" : ""}</button>
          );
        })}
        <span style={{ flex: 1 }} />
        <span style={{ fontSize: 11.5, color: "#9AA3AD" }}>用于按钮、选中态与强调元素</span>
      </div>

      <GroupTitle>动效</GroupTitle>
      <Card>
        <Row first title="减少动效" desc="减弱过渡与位移动画,降低视觉干扰">
          <Toggle on={s.reduceMotion} onChange={v => set({ reduceMotion: v })} />
        </Row>
      </Card>
    </div>
  );
}

function Typography({ s, set }) {
  return (
    <div style={{ animation: "fslide .16s ease" }}>
      <GroupTitle first>字体</GroupTitle>
      <Card>
        <Row first title="界面字体" desc="应用于所有界面文本">
          <Segmented value={s.uiFont} onChange={v => set({ uiFont: v })}
            options={[["system", "系统默认"], ["sans", "无衬线"], ["mono", "等宽"]]} />
        </Row>
        <div style={{ padding: "14px 16px", borderTop: "1px solid #F0F3F6" }}>
          <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: "#1C2530" }}>字体大小</div>
              <div style={{ fontSize: 11.5, color: "#8A939D", marginTop: 2 }}>调整全局文字与控件尺寸</div>
            </div>
            <Segmented value={s.fontScale} onChange={v => set({ fontScale: v })}
              options={FONT_SIZES.map(([v, l]) => [v, l])} />
          </div>
          <div style={{ marginTop: 13, border: "1px solid #EEF1F4", borderRadius: 9,
            background: "#F8FAFB", padding: "13px 15px" }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: "#1C2530" }}>预览</div>
            <div style={{ fontSize: 12.5, color: "#5B6672", marginTop: 4, lineHeight: 1.55 }}>
              将「重构支付网关重试逻辑」会话迁移至 Codex CLI,并在写入后运行探针验收。</div>
          </div>
        </div>
      </Card>
    </div>
  );
}

function About() {
  const links = [["帮助文档", "查看"], ["反馈问题", "查看"], ["开源许可", "查看"]];
  return (
    <div style={{ animation: "fslide .16s ease" }}>
      <div style={{ border: "1px solid #E7ECF0", borderRadius: 12, background: "#fff", padding: 20,
        display: "flex", alignItems: "center", gap: 15 }}>
        <img className="noinvert" src={appIcon} alt="Ferry" width={56} height={56}
          style={{ borderRadius: 14, flex: "none", display: "block" }} />
        <div>
          <div style={{ fontSize: 16, fontWeight: 650, color: "#1C2530" }}>Ferry</div>
          <div className="mono" style={{ fontSize: 12, color: "#8A939D", marginTop: 2 }}>
            版本 {VERSION}</div>
          <div style={{ fontSize: 12, color: "#6B7682", marginTop: 6, lineHeight: 1.5 }}>
            在本机各 AI 编码工具之间迁移与管理会话的桌面工具。</div>
        </div>
      </div>

      <GroupTitle>隐私</GroupTitle>
      <div style={{ border: "1px solid #E7ECF0", borderRadius: 12, background: "#fff",
        padding: "15px 16px", fontSize: 12.5, color: "#40494F", lineHeight: 1.6 }}>
        Ferry 完全在本机运行,会话与快照数据不会离开这台设备,也不会上传到任何服务器。
        探针验收会真实调用一次目标工具的模型,除此之外没有任何出网行为。</div>

      <GroupTitle>资源</GroupTitle>
      <Card>
        {links.map(([label, action], i) => (
          <div key={label} style={{ display: "flex", alignItems: "center", padding: "13px 16px",
            borderTop: i === 0 ? "none" : "1px solid #F0F3F6" }}>
            <span style={{ flex: 1, fontSize: 13, color: "#1C2530" }}>{label}</span>
            <span style={{ fontSize: 12.5, color: "#9AA3AD" }}>{action}</span>
          </div>
        ))}
      </Card>
    </div>
  );
}

// ---------- 页面外壳 ----------
export default function SettingsPage({ settings, setSettings, scan, env, scanning, onRescan,
  guideSeen, onOpenGuide, onFirstRun, onClose }) {
  const [section, setSection] = useState("general");
  const title = Object.fromEntries(SECTIONS)[section];

  return (
    <div style={{ position: "absolute", inset: 0, zIndex: 60, display: "flex",
      background: "#F6F8FA", animation: "ffade .14s ease" }}>
      {/* 分类栏 */}
      <div style={{ width: 212, flex: "none", background: "#EEF1F4", borderRight: "1px solid #E1E7EC",
        display: "flex", flexDirection: "column", padding: "16px 12px" }}>
        <div style={{ fontSize: 11, fontWeight: 700, color: "#9AA3AD", letterSpacing: ".08em",
          padding: "2px 8px 12px" }}>设置</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {SECTIONS.map(([k, label]) => {
            const on = section === k;
            return (
              <button key={k} className={on ? undefined : "hov-item"} onClick={() => setSection(k)}
                style={{ display: "flex", alignItems: "center", gap: 11, height: 36, padding: "0 11px",
                  border: "none", borderRadius: 8, background: on ? "#fff" : "transparent",
                  color: on ? "#1C2530" : "#4A545E", fontSize: 13, fontWeight: on ? 650 : 500,
                  cursor: "pointer", textAlign: "left" }}>
                <SetGlyph name={k} color={on ? "var(--accent)" : "#6B7682"} />{label}
              </button>
            );
          })}
        </div>
        <div style={{ flex: 1 }} />
        <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "9px 10px",
          borderRadius: 10, background: "#E4E8EC" }}>
          <img className="noinvert" src={appIcon} alt="Ferry" width={28} height={28}
            style={{ borderRadius: 7, flex: "none", display: "block" }} />
          <div style={{ lineHeight: 1.3 }}>
            <div style={{ fontSize: 12, fontWeight: 600, color: "#334155" }}>Ferry</div>
            <div style={{ fontSize: 10.5, color: "#9AA3AD" }}>版本 {VERSION}</div>
          </div>
        </div>
      </div>

      {/* 内容 */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
        <div style={{ height: 56, flex: "none", display: "flex", alignItems: "center", gap: 12,
          padding: "0 22px", borderBottom: "1px solid #E7ECF0", background: "#FBFCFD" }}>
          <div style={{ fontSize: 16, fontWeight: 650, color: "#1C2530" }}>{title}</div>
          <div style={{ flex: 1 }} />
          <button className="hov" onClick={onClose} title="关闭设置 (Esc)"
            style={{ width: 30, height: 30, borderRadius: "50%", border: "none",
              background: "#EEF1F4", color: "#6B7682", cursor: "pointer",
              display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
            <svg viewBox="0 0 14 14" style={{ width: 13, height: 13 }}>
              <line x1="3" y1="3" x2="11" y2="11" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
              <line x1="11" y1="3" x2="3" y2="11" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
          </button>
        </div>
        <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "22px 26px" }}>
          <div style={{ maxWidth: 640 }}>
            {section === "general" && <General guideSeen={guideSeen}
              onOpenGuide={onOpenGuide} onFirstRun={onFirstRun} />}
            {section === "sources" && <Sources scan={scan} env={env}
              scanning={scanning} onRescan={onRescan} />}
            {section === "appearance" && <Appearance s={settings} set={setSettings} />}
            {section === "type" && <Typography s={settings} set={setSettings} />}
            {section === "about" && <About />}
          </div>
        </div>
      </div>
    </div>
  );
}
