// 设置悬浮弹窗(参考 LM Studio):左侧分类 + 偏好设置 / 数据来源
import { useState } from "react";
import { TOOL_NAME, TOOLS } from "../../api/contract/tools.js";
import { SetGlyph, ToolIcon } from "../../components/ui/icons.jsx";
import { formatBytes } from "./useAppUpdater.js";

const SECTIONS = [["prefs", "偏好设置"], ["sources", "数据来源"], ["updates", "软件更新"]];

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

      <GroupTitle>写入验收</GroupTitle>
      <Card>
        <Row first title="运行时探针"
          desc="写入后在临时影子会话上真实 resume 验证,消耗一次模型调用,完成后自动清理,不向正式会话追加消息;关闭时仅做结构验证(默认)">
          <Toggle on={s.runtimeProbe} onChange={v => set({ runtimeProbe: v })} />
        </Row>
      </Card>

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

const UPDATE_COPY = {
  idle: "尚未检查更新",
  checking: "正在检查更新…",
  upToDate: "当前已是最新版本",
  available: "发现新版本，等待确认下载",
  downloading: "正在下载更新",
  downloaded: "更新已下载，等待确认安装",
  installing: "正在安装，即将重新启动…",
  error: "更新操作失败",
};

function Updates({ s, set, updater }) {
  const { phase, currentVersion, update, downloaded, total, error, failedAction, supported,
    checkForUpdate, downloadUpdate, installAndRestart } = updater;
  const checking = phase === "checking";
  const downloading = phase === "downloading";
  const progress = total ? Math.min(100, downloaded / total * 100) : null;
  const canCheck = supported && !["checking", "downloading", "installing"].includes(phase);

  return (
    <div style={{ animation: "fslide .16s ease" }}>
      <GroupTitle first>版本</GroupTitle>
      <Card>
        <Row first title="当前版本" desc={supported ? "Ferry 桌面应用" : "浏览器预览不支持应用内更新"}>
          <span className="mono" style={{ fontSize: 12, color: "var(--tx3b)" }}>v{currentVersion}</span>
        </Row>
        <Row title="自动检查更新" desc="启动后延迟检查；不会自动下载或安装">
          <Toggle on={s.autoCheckUpdates} onChange={v => set({ autoCheckUpdates: v })} />
        </Row>
      </Card>

      <GroupTitle>更新状态</GroupTitle>
      <Card>
        <div aria-live="polite" aria-busy={checking || downloading || phase === "installing"}
          style={{ padding: 16 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
            <div style={{ flex: "1 1 240px", minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 650, color: error ? "var(--err-deep)" : "var(--tx1)" }}>
                {UPDATE_COPY[phase] || UPDATE_COPY.idle}
              </div>
              {update && <div style={{ fontSize: 11.5, color: "var(--tx4)", marginTop: 3 }}>
                v{currentVersion} → v{update.version}{update.date ? ` · ${new Date(update.date).toLocaleDateString()}` : ""}
              </div>}
              {error && <div style={{ fontSize: 11.5, color: "var(--err-deep)", marginTop: 5,
                overflowWrap: "anywhere" }}>{error}</div>}
            </div>
            <div className="update-actions" style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
              {(phase === "idle" || phase === "upToDate" || (phase === "error" && failedAction === "check")) &&
                <button className="fbtn" onClick={checkForUpdate} disabled={!canCheck}
                  style={{ height: 30, fontSize: 12 }}>{failedAction === "check" ? "重试检查" : "检查更新"}</button>}
              {(phase === "available" || (phase === "error" && failedAction === "download")) &&
                <button className="fbtn-primary" onClick={downloadUpdate}
                  style={{ height: 30, padding: "0 13px" }}>{failedAction === "download" ? "重试下载" : "下载更新"}</button>}
              {(phase === "downloaded" || (phase === "error" && failedAction === "install")) &&
                <button className="fbtn-primary" onClick={installAndRestart}
                  style={{ height: 30, padding: "0 13px" }}>{failedAction === "install" ? "重试安装并重启" : "安装并重启"}</button>}
            </div>
          </div>

          {downloading && <div style={{ marginTop: 14 }}>
            <div className={`update-progress ${progress == null ? "indeterminate" : ""}`}
              role="progressbar" aria-label="更新下载进度" aria-valuemin={0} aria-valuemax={100}
              {...(progress == null ? {} : { "aria-valuenow": Math.round(progress) })}>
              <span style={progress == null ? undefined : { width: `${progress}%` }} />
            </div>
            <div className="mono" style={{ fontSize: 11, color: "var(--tx5)", marginTop: 7 }}>
              {formatBytes(downloaded)} / {total == null ? "大小未知" : formatBytes(total)}
              {progress != null ? ` · ${Math.round(progress)}%` : ""}
            </div>
          </div>}
        </div>
        {update?.body && <div style={{ borderTop: "1px solid var(--line6)", padding: "14px 16px" }}>
          <div style={{ fontSize: 11.5, fontWeight: 700, color: "var(--tx3b)", marginBottom: 7 }}>版本说明</div>
          <div style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere", maxHeight: 180,
            overflowY: "auto", fontSize: 12, lineHeight: 1.6, color: "var(--tx3b)" }}>{update.body}</div>
        </div>}
      </Card>
      <div style={{ fontSize: 11, color: "var(--tx5)", marginTop: 10, lineHeight: 1.55, paddingLeft: 2 }}>
        Ferry 会分别请求你的下载与安装确认。只有下载完成后才可安装并重新启动。
      </div>
    </div>
  );
}

// ---------- 弹窗外壳 ----------
export default function SettingsPage({ settings, setSettings, scan, env, scanning, onRescan,
  updater, guideSeen, onOpenGuide, onFirstRun, onClose }) {
  const [section, setSection] = useState("prefs");
  const title = Object.fromEntries(SECTIONS)[section];

  return (
    <div onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}
      style={{ position: "absolute", inset: 0, zIndex: 60, display: "flex", alignItems: "center",
        justifyContent: "center", background: "var(--scrim)", animation: "ffade .14s ease" }}>
      <div className="settings-sheet" style={{ width: "min(860px, calc(100vw - 40px))", height: "min(600px, calc(100vh - 48px))",
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
              {section === "updates" && <Updates s={settings} set={setSettings} updater={updater} />}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
