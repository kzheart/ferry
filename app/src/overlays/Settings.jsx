// 设置悬浮弹窗(参考 LM Studio):左侧分类 + 偏好设置 / 数据来源
import { useState } from "react";
import { TOOL_NAME, TOOLS } from "../api.js";
import { SetGlyph, ToolIcon } from "../icons.jsx";
import appIcon from "../assets/app-icon.png";

const VERSION = typeof __APP_VERSION__ !== "undefined" ? __APP_VERSION__ : "dev";

const SECTIONS = [["prefs", "偏好设置"], ["sources", "数据来源"]];

// ---------- 通用排版件 ----------
const GroupTitle = ({ children, first }) => (
  <div style={{ fontSize: 11.5, fontWeight: 700, color: "var(--tx5)", letterSpacing: ".05em",
    margin: first ? "0 0 9px 2px" : "22px 0 9px 2px" }}>{children}</div>
);

const Card = ({ children }) => (
  <div style={{ border: "1px solid var(--line4)", borderRadius: 12, background: "var(--surface)",
    overflow: "hidden" }}>{children}</div>
);

function Row({ title, desc, children, first }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 12, padding: "14px 16px",
      borderTop: first ? "none" : "1px solid var(--line6)" }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: "var(--tx1)" }}>{title}</div>
        {desc && <div style={{ fontSize: 11.5, color: "var(--tx4)", marginTop: 2 }}>{desc}</div>}
      </div>
      {children}
    </div>
  );
}

function Toggle({ on, onChange }) {
  return (
    <button onClick={() => onChange(!on)} aria-pressed={on}
      style={{ width: 44, height: 26, borderRadius: 20, border: "none", flex: "none",
        background: on ? "var(--accent)" : "var(--toggle-off)", cursor: "pointer", padding: 0,
        position: "relative", transition: "background .15s ease" }}>
      <span style={{ position: "absolute", top: 3, left: on ? 21 : 3, width: 20, height: 20,
        borderRadius: "50%", background: "var(--surface)", boxShadow: "0 1px 3px rgba(0,0,0,.28)",
        transition: "left .15s ease" }} />
    </button>
  );
}

// ---------- 偏好设置 ----------
function Prefs({ s, set, guideSeen, onOpenGuide, onFirstRun }) {
  const themes = [
    ["light", "浅色", "#FBFCFD"],
    ["dark", "深色", "#17171A"],
    ["system", "跟随系统", "linear-gradient(105deg,#FBFCFD 0 50%,#17171A 50% 100%)"],
  ];
  return (
    <div style={{ animation: "fslide .16s ease" }}>
      <GroupTitle first>外观</GroupTitle>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 12 }}>
        {themes.map(([k, label, sw]) => {
          const on = s.theme === k;
          return (
            <div key={k} onClick={() => set({ theme: k })}
              style={{ border: `1.5px solid ${on ? "var(--accent)" : "var(--line4)"}`,
                background: on ? "var(--acc-soft6)" : "var(--surface)", borderRadius: 12, padding: 12,
                cursor: "pointer" }}>
              <div style={{ height: 54, borderRadius: 8, background: sw,
                border: "1px solid rgba(20,28,38,.10)" }} />
              <div style={{ display: "flex", alignItems: "center", gap: 7, marginTop: 11 }}>
                <span style={{ width: 15, height: 15, borderRadius: "50%", flex: "none",
                  border: `2px solid ${on ? "var(--accent)" : "var(--toggle-off)"}`, display: "inline-flex",
                  alignItems: "center", justifyContent: "center" }}>
                  <span style={{ width: 7, height: 7, borderRadius: "50%",
                    background: on ? "var(--accent)" : "transparent" }} /></span>
                <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx2)" }}>{label}</span>
              </div>
            </div>
          );
        })}
      </div>

      <GroupTitle>动效</GroupTitle>
      <Card>
        <Row first title="减少动效" desc="减弱过渡与位移动画,降低视觉干扰">
          <Toggle on={s.reduceMotion} onChange={v => set({ reduceMotion: v })} />
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

// ---------- 数据来源 ----------
function Sources({ scan, env, scanning, onRescan }) {
  const tools = scan?.tools || {};
  const connected = TOOLS.filter(t => tools[t]?.ok).length;
  const total = TOOLS.reduce((a, t) => a + (tools[t]?.count || 0), 0);
  return (
    <div style={{ animation: "fslide .16s ease" }}>
      <div style={{ display: "flex", alignItems: "flex-end", margin: "0 0 9px 2px" }}>
        <div style={{ flex: 1, fontSize: 11.5, fontWeight: 700, color: "var(--tx5)",
          letterSpacing: ".05em" }}>已连接的工具</div>
        <div style={{ fontSize: 11.5, color: "var(--tx4)" }}>
          {connected} 个已连接 · {total} 个会话</div>
      </div>
      <Card>
        {TOOLS.map((t, i) => {
          const info = tools[t] || {};
          const ok = info.ok;
          return (
            <div key={t} style={{ display: "flex", alignItems: "center", gap: 13,
              padding: "14px 16px", borderTop: i === 0 ? "none" : "1px solid var(--line6)" }}>
              <ToolIcon tool={t} size={30} dot={ok ? "var(--ok)" : "var(--err)"} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: "var(--tx1)" }}>
                  {TOOL_NAME[t]}
                  {env?.[t]?.version && <span style={{ fontWeight: 400, color: "var(--tx5)",
                    fontSize: 11.5 }}> · v{env[t].version}</span>}
                </div>
                <div className="mono" style={{ fontSize: 11, color: "var(--tx5)", marginTop: 2,
                  whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                  {info.path || "—"}</div>
              </div>
              <div style={{ textAlign: "right", flex: "none", marginRight: 4 }}>
                <div style={{ fontSize: 12, color: "var(--tx3b)" }}>
                  {ok ? `${info.count} 个会话` : (info.error || "不可用")}</div>
                <div style={{ display: "inline-flex", alignItems: "center", gap: 5, fontSize: 11.5,
                  fontWeight: 600, color: ok ? "var(--ok-deep)" : "var(--err-deep)", marginTop: 2 }}>
                  <span style={{ width: 6, height: 6, borderRadius: "50%",
                    background: ok ? "var(--ok)" : "var(--err)" }} />{ok ? "已连接" : "扫描失败"}</div>
              </div>
              <button className="fbtn" style={{ height: 30, fontSize: 12, flex: "none" }}
                onClick={onRescan} disabled={scanning}>
                {scanning ? "扫描中…" : "重新扫描"}</button>
            </div>
          );
        })}
      </Card>
      <div style={{ fontSize: 11, color: "var(--tx5)", marginTop: 10, lineHeight: 1.55,
        paddingLeft: 2 }}>
        Ferry 在本机自动发现受支持工具的会话目录。源会话保持只读,不会被修改。</div>
    </div>
  );
}

// ---------- 弹窗外壳 ----------
export default function SettingsPage({ settings, setSettings, scan, env, scanning, onRescan,
  guideSeen, onOpenGuide, onFirstRun, onClose }) {
  const [section, setSection] = useState("prefs");
  const title = Object.fromEntries(SECTIONS)[section];

  return (
    <div onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}
      style={{ position: "absolute", inset: 0, zIndex: 60, display: "flex", alignItems: "center",
        justifyContent: "center", background: "var(--scrim)", animation: "ffade .14s ease" }}>
      <div style={{ width: "min(860px, calc(100vw - 96px))", height: "min(600px, calc(100vh - 120px))",
        display: "flex", borderRadius: 14, overflow: "hidden", background: "var(--settings-bg)",
        border: "1px solid var(--line)", boxShadow: "0 24px 64px -16px rgba(0,0,0,.45)",
        animation: "fsheet .18s ease" }}>
        {/* 分类栏 */}
        <div style={{ width: 196, flex: "none", background: "var(--settings-rail)",
          borderRight: "1px solid var(--line)", display: "flex", flexDirection: "column",
          padding: "16px 12px" }}>
          <div style={{ fontSize: 11, fontWeight: 700, color: "var(--tx5)", letterSpacing: ".08em",
            padding: "2px 8px 12px" }}>设置</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            {SECTIONS.map(([k, label]) => {
              const on = section === k;
              return (
                <button key={k} className={on ? undefined : "hov-item"} onClick={() => setSection(k)}
                  style={{ display: "flex", alignItems: "center", gap: 11, height: 36, padding: "0 11px",
                    border: "none", borderRadius: 8, background: on ? "var(--seg-on)" : "transparent",
                    color: on ? "var(--tx1)" : "var(--tx2b)", fontSize: 13, fontWeight: on ? 650 : 500,
                    cursor: "pointer", textAlign: "left" }}>
                  <SetGlyph name={k} color={on ? "var(--tx1)" : "var(--tx3b)"} />{label}
                </button>
              );
            })}
          </div>
          <div style={{ flex: 1 }} />
          <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "9px 10px",
            borderRadius: 10, background: "var(--badge)" }}>
            <img className="noinvert" src={appIcon} alt="Ferry" width={28} height={28}
              style={{ borderRadius: 7, flex: "none", display: "block" }} />
            <div style={{ lineHeight: 1.3 }}>
              <div style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2)" }}>Ferry</div>
              <div className="mono" style={{ fontSize: 10.5, color: "var(--tx5)" }}>v{VERSION}</div>
            </div>
          </div>
        </div>

        {/* 内容 */}
        <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
          <div style={{ height: 54, flex: "none", display: "flex", alignItems: "center", gap: 12,
            padding: "0 20px", borderBottom: "1px solid var(--line4)" }}>
            <div style={{ fontSize: 15, fontWeight: 650, color: "var(--tx1)" }}>{title}</div>
            <div style={{ flex: 1 }} />
            <button className="hov" onClick={onClose} title="关闭设置 (Esc)"
              style={{ width: 28, height: 28, borderRadius: "50%", border: "none",
                background: "var(--fill4)", color: "var(--tx3b)", cursor: "pointer",
                display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
              <svg viewBox="0 0 14 14" style={{ width: 12, height: 12 }}>
                <line x1="3" y1="3" x2="11" y2="11" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
                <line x1="11" y1="3" x2="3" y2="11" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
              </svg>
            </button>
          </div>
          <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "20px 24px" }}>
            <div style={{ maxWidth: 620 }}>
              {section === "prefs" && <Prefs s={settings} set={setSettings} guideSeen={guideSeen}
                onOpenGuide={onOpenGuide} onFirstRun={onFirstRun} />}
              {section === "sources" && <Sources scan={scan} env={env}
                scanning={scanning} onRescan={onRescan} />}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
