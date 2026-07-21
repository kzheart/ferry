// 设置悬浮弹窗(参考 LM Studio):左侧分类 + 偏好设置 / 数据来源
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOL_NAME, TOOLS } from "../../api/contract/tools.js";
import { LOCALE_META } from "../../i18n/index.js";
import { SetGlyph, ToolIcon } from "../../components/ui/icons.jsx";
import { formatBytes } from "./useAppUpdater.js";

const SECTIONS = [["prefs", "settings:sections.prefs"], ["sources", "settings:sections.sources"], ["updates", "settings:sections.updates"]];

// ---------- 通用排版件 ----------
const GroupTitle = ({ children, first }) => (
  <div style={{ fontSize: 11, fontWeight: 700, color: "var(--tx5)", letterSpacing: ".05em",
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
        {desc && <div style={{ fontSize: 11, color: "var(--tx4)", marginTop: 2 }}>{desc}</div>}
      </div>
      {children}
    </div>
  );
}

// 原生 select:自带键盘导航与系统弹层,选项多了也不会撑爆设置面板
function Select({ value, onChange, children }) {
  return (
    <div style={{ position: "relative", flex: "none" }}>
      <select value={value} onChange={e => onChange(e.target.value)}
        style={{ appearance: "none", height: 30, padding: "0 28px 0 11px", borderRadius: 8,
          border: "1px solid var(--line4)", background: "var(--surface)", color: "var(--tx1)",
          fontSize: 12, fontWeight: 600, fontFamily: "inherit", cursor: "default" }}>
        {children}
      </select>
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden
        style={{ position: "absolute", right: 10, top: "50%", transform: "translateY(-50%)",
          pointerEvents: "none", color: "var(--tx4)" }}>
        <path d="M2 4l3 3 3-3" fill="none" stroke="currentColor" strokeWidth="1.6"
          strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    </div>
  );
}

function Toggle({ on, onChange }) {
  return (
    <button onClick={() => onChange(!on)} aria-pressed={on}
      style={{ width: 44, height: 26, borderRadius: 20, border: "none", flex: "none",
        background: on ? "var(--accent)" : "var(--toggle-off)", cursor: "default", padding: 0,
        position: "relative", transition: "background .15s ease" }}>
      <span style={{ position: "absolute", top: 3, left: on ? 21 : 3, width: 20, height: 20,
        borderRadius: "50%", background: "var(--surface)", boxShadow: "0 1px 3px rgba(0,0,0,.28)",
        transition: "left .15s ease" }} />
    </button>
  );
}

// ---------- 偏好设置 ----------
function Prefs({ s, set, guideSeen, onOpenGuide, onFirstRun }) {
  const { t } = useTranslation();
  const themes = [
    ["light", t("settings:theme.light"), "#FBFCFD"],
    ["dark", t("settings:theme.dark"), "#17171A"],
    ["system", t("settings:theme.system"), "linear-gradient(105deg,#FBFCFD 0 50%,#17171A 50% 100%)"],
  ];
  const localeValue = s.locale ?? "";
  return (
    <div style={{  }}>
      <GroupTitle first>{t("settings:theme.groupTitle")}</GroupTitle>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 12 }}>
        {themes.map(([k, label, sw]) => {
          const on = s.theme === k;
          return (
            <div key={k} onClick={() => set({ theme: k })}
              style={{ border: `1.5px solid ${on ? "var(--accent)" : "var(--line4)"}`,
                background: on ? "var(--acc-soft6)" : "var(--surface)", borderRadius: 12, padding: 12,
                cursor: "default" }}>
              <div style={{ height: 54, borderRadius: 8, background: sw,
                border: "1px solid rgba(20,28,38,.10)" }} />
              <div style={{ display: "flex", alignItems: "center", gap: 7, marginTop: 11 }}>
                <span style={{ width: 15, height: 15, borderRadius: "50%", flex: "none",
                  border: `2px solid ${on ? "var(--accent)" : "var(--toggle-off)"}`, display: "inline-flex",
                  alignItems: "center", justifyContent: "center" }}>
                  <span style={{ width: 7, height: 7, borderRadius: "50%",
                    background: on ? "var(--accent)" : "transparent" }} /></span>
                <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2)" }}>{label}</span>
              </div>
            </div>
          );
        })}
      </div>

      <GroupTitle>{t("language.label")}</GroupTitle>
      <Card>
        <Row first title={t("language.label")}
          desc={localeValue ? undefined : t("settings:sections.followSystemDesc")}>
          <Select value={localeValue}
            onChange={v => set({ locale: v || null })}>
            <option value="">{t("language.followSystem")}</option>
            {LOCALE_META.map(l => (
              <option key={l.code} value={l.code}>{l.nativeName}</option>
            ))}
          </Select>
        </Row>
      </Card>

      <GroupTitle>{t("settings:writeCheck.groupTitle")}</GroupTitle>
      <Card>
        <Row first title={t("settings:writeCheck.runtimeProbe")}
          desc={t("settings:writeCheck.runtimeProbeDesc")}>
          <Toggle on={s.runtimeProbe} onChange={v => set({ runtimeProbe: v })} />
        </Row>
      </Card>

      <GroupTitle>{t("settings:motion.groupTitle")}</GroupTitle>
      <Card>
        <Row first title={t("settings:motion.reduceMotion")} desc={t("settings:motion.reduceMotionDesc")}>
          <Toggle on={s.reduceMotion} onChange={v => set({ reduceMotion: v })} />
        </Row>
      </Card>

      <GroupTitle>{t("settings:guideSection.groupTitle")}</GroupTitle>
      <Card>
        <Row first title={t("settings:guideSection.guide")} desc={t("settings:guideSection.guideDesc")}>
          <button className="fbtn-primary" style={{ height: 30, padding: "0 13px" }}
            onClick={onOpenGuide}>{guideSeen ? t("settings:guideSection.reviewGuide") : t("settings:guideSection.quickStart")}</button>
        </Row>
        <Row title={t("settings:guideSection.firstRun")} desc={t("settings:guideSection.firstRunDesc")}>
          <button className="fbtn" style={{ height: 30, fontSize: 12 }}
            onClick={onFirstRun}>{t("settings:guideSection.open")}</button>
        </Row>
      </Card>
    </div>
  );
}

// ---------- 数据来源 ----------
function Sources({ scan, env, scanning, onRescan }) {
  const { t } = useTranslation();
  const tools = scan?.tools || {};
  const connected = TOOLS.filter(t2 => tools[t2]?.ok).length;
  const total = TOOLS.reduce((a, t2) => a + (tools[t2]?.count || 0), 0);
  return (
    <div style={{  }}>
      <div style={{ display: "flex", alignItems: "flex-end", margin: "0 0 9px 2px" }}>
        <div style={{ flex: 1, fontSize: 11, fontWeight: 700, color: "var(--tx5)",
          letterSpacing: ".05em" }}>{t("settings:sources.connectedTools")}</div>
        <div style={{ fontSize: 11, color: "var(--tx4)" }}>
          {t("settings:sources.connectedMeta", { connected, total })}</div>
      </div>
      <Card>
        {TOOLS.map((t2, i) => {
          const info = tools[t2] || {};
          const ok = info.ok;
          return (
            <div key={t2} style={{ display: "flex", alignItems: "center", gap: 13,
              padding: "14px 16px", borderTop: i === 0 ? "none" : "1px solid var(--line6)" }}>
              <ToolIcon tool={t2} size={30} dot={ok ? "var(--ok)" : "var(--err)"} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: "var(--tx1)" }}>
                  {TOOL_NAME[t2]}
                  {env?.[t2]?.version && <span style={{ fontWeight: 400, color: "var(--tx5)",
                    fontSize: 11 }}> · v{env[t2].version}</span>}
                </div>
                <div className="mono" style={{ fontSize: 11, color: "var(--tx5)", marginTop: 2,
                  whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                  {info.path || "—"}</div>
              </div>
              <div style={{ textAlign: "right", flex: "none", marginRight: 4 }}>
                <div style={{ fontSize: 12, color: "var(--tx3b)" }}>
                  {ok ? t("settings:sources.sessionsCount", { n: info.count }) : (info.error || t("settings:sources.unavailable"))}</div>
                <div style={{ display: "inline-flex", alignItems: "center", gap: 5, fontSize: 11,
                  fontWeight: 600, color: ok ? "var(--ok-deep)" : "var(--err-deep)", marginTop: 2 }}>
                  <span style={{ width: 6, height: 6, borderRadius: "50%",
                    background: ok ? "var(--ok)" : "var(--err)" }} />{ok ? t("settings:sources.connected") : t("settings:sources.scanFailed")}</div>
              </div>
              <button className="fbtn" style={{ height: 30, fontSize: 12, flex: "none" }}
                onClick={onRescan} disabled={scanning}>
                {scanning ? t("settings:sources.scanning") : t("settings:sources.rescan")}</button>
            </div>
          );
        })}
      </Card>
      <div style={{ fontSize: 11, color: "var(--tx5)", marginTop: 10, lineHeight: 1.55,
        paddingLeft: 2 }}>
        {t("settings:sources.footnote")}</div>
    </div>
  );
}

const UPDATE_COPY_KEY = {
  idle: "settings:updates.phase.idle",
  checking: "settings:updates.phase.checking",
  upToDate: "settings:updates.phase.upToDate",
  available: "settings:updates.phase.available",
  downloading: "settings:updates.phase.downloading",
  downloaded: "settings:updates.phase.downloaded",
  installing: "settings:updates.phase.installing",
  error: "settings:updates.phase.error",
};

function Updates({ s, set, updater }) {
  const { t, i18n } = useTranslation();
  const { phase, currentVersion, update, downloaded, total, error, failedAction, supported,
    checkForUpdate, downloadUpdate, installAndRestart } = updater;
  const checking = phase === "checking";
  const downloading = phase === "downloading";
  const progress = total ? Math.min(100, downloaded / total * 100) : null;
  const canCheck = supported && !["checking", "downloading", "installing"].includes(phase);

  return (
    <div style={{  }}>
      <GroupTitle first>{t("settings:updates.groupVersion")}</GroupTitle>
      <Card>
        <Row first title={t("settings:updates.currentVersion")}
          desc={supported ? t("settings:updates.currentVersionDescDesktop") : t("settings:updates.currentVersionDescWeb")}>
          <span className="mono" style={{ fontSize: 12, color: "var(--tx3b)" }}>v{currentVersion}</span>
        </Row>
        <Row title={t("settings:updates.autoCheck")} desc={t("settings:updates.autoCheckDesc")}>
          <Toggle on={s.autoCheckUpdates} onChange={v => set({ autoCheckUpdates: v })} />
        </Row>
      </Card>

      <GroupTitle>{t("settings:updates.groupStatus")}</GroupTitle>
      <Card>
        <div aria-live="polite" aria-busy={checking || downloading || phase === "installing"}
          style={{ padding: 16 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
            <div style={{ flex: "1 1 240px", minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 650, color: error ? "var(--err-deep)" : "var(--tx1)" }}>
                {t(UPDATE_COPY_KEY[phase] || UPDATE_COPY_KEY.idle)}
              </div>
              {update && <div style={{ fontSize: 11, color: "var(--tx4)", marginTop: 3 }}>
                v{currentVersion} → v{update.version}{update.date ? ` · ${new Date(update.date).toLocaleDateString(i18n.language)}` : ""}
              </div>}
              {error && <div style={{ fontSize: 11, color: "var(--err-deep)", marginTop: 5,
                overflowWrap: "anywhere" }}>{error}</div>}
            </div>
            <div className="update-actions" style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
              {(phase === "idle" || phase === "upToDate" || (phase === "error" && failedAction === "check")) &&
                <button className="fbtn" onClick={checkForUpdate} disabled={!canCheck}
                  style={{ height: 30, fontSize: 12 }}>{failedAction === "check" ? t("settings:updates.retryCheck") : t("settings:updates.check")}</button>}
              {(phase === "available" || (phase === "error" && failedAction === "download")) &&
                <button className="fbtn-primary" onClick={downloadUpdate}
                  style={{ height: 30, padding: "0 13px" }}>{failedAction === "download" ? t("settings:updates.retryDownload") : t("settings:updates.download")}</button>}
              {(phase === "downloaded" || (phase === "error" && failedAction === "install")) &&
                <button className="fbtn-primary" onClick={installAndRestart}
                  style={{ height: 30, padding: "0 13px" }}>{failedAction === "install" ? t("settings:updates.retryInstall") : t("settings:updates.installRestart")}</button>}
            </div>
          </div>

          {downloading && <div style={{ marginTop: 14 }}>
            <div className={`update-progress ${progress == null ? "indeterminate" : ""}`}
              role="progressbar" aria-label={t("settings:updates.downloadProgress")} aria-valuemin={0} aria-valuemax={100}
              {...(progress == null ? {} : { "aria-valuenow": Math.round(progress) })}>
              <span style={progress == null ? undefined : { width: `${progress}%` }} />
            </div>
            <div className="mono" style={{ fontSize: 11, color: "var(--tx5)", marginTop: 7 }}>
              {formatBytes(downloaded)} / {total == null ? t("settings:updates.sizeUnknown") : formatBytes(total)}
              {progress != null ? ` · ${Math.round(progress)}%` : ""}
            </div>
          </div>}
        </div>
        {update?.body && <div style={{ borderTop: "1px solid var(--line6)", padding: "14px 16px" }}>
          <div style={{ fontSize: 11, fontWeight: 700, color: "var(--tx3b)", marginBottom: 7 }}>{t("settings:updates.versionNotes")}</div>
          <div style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere", maxHeight: 180,
            overflowY: "auto", fontSize: 12, lineHeight: 1.6, color: "var(--tx3b)" }}>{update.body}</div>
        </div>}
      </Card>
      <div style={{ fontSize: 11, color: "var(--tx5)", marginTop: 10, lineHeight: 1.55, paddingLeft: 2 }}>
        {t("settings:updates.footnote")}</div>
    </div>
  );
}

// ---------- 弹窗外壳 ----------
export default function SettingsPage({ settings, setSettings, scan, env, scanning, onRescan,
  updater, guideSeen, onOpenGuide, onFirstRun, onClose }) {
  const { t } = useTranslation();
  const [section, setSection] = useState("prefs");
  const title = Object.fromEntries(SECTIONS)[section];

  return (
    <div onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}
      style={{ position: "absolute", inset: 0, zIndex: 60, display: "flex", alignItems: "center",
        justifyContent: "center", background: "var(--scrim)" }}>
      <div className="settings-sheet" style={{ width: "min(860px, calc(100vw - 40px))", height: "min(600px, calc(100vh - 48px))",
        display: "flex", borderRadius: 14, overflow: "hidden", background: "var(--settings-bg)",
        border: "1px solid var(--line)", boxShadow: "var(--shadow-sheet)",
         }}>
        {/* 分类栏 */}
        <div style={{ width: 196, flex: "none", background: "var(--settings-rail)",
          borderRight: "1px solid var(--line)", display: "flex", flexDirection: "column",
          padding: "16px 12px" }}>
          <div style={{ fontSize: 11, fontWeight: 700, color: "var(--tx5)", letterSpacing: ".08em",
            padding: "2px 8px 12px" }}>{t("settings:sections.railTitle")}</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            {SECTIONS.map(([k, labelKey]) => {
              const on = section === k;
              return (
                <button key={k} className={on ? undefined : "hov-item"} onClick={() => setSection(k)}
                  style={{ display: "flex", alignItems: "center", gap: 11, height: 36, padding: "0 11px",
                    border: "none", borderRadius: 8, background: on ? "var(--seg-on)" : "transparent",
                    color: on ? "var(--tx1)" : "var(--tx2b)", fontSize: 13, fontWeight: on ? 650 : 500,
                    cursor: "default", textAlign: "left" }}>
                  <SetGlyph name={k} color={on ? "var(--tx1)" : "var(--tx3b)"} />{t(labelKey)}
                </button>
              );
            })}
          </div>
        </div>

        {/* 内容 */}
        <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
          <div style={{ height: 54, flex: "none", display: "flex", alignItems: "center", gap: 12,
            padding: "0 20px", borderBottom: "1px solid var(--line4)" }}>
            <div style={{ fontSize: 15, fontWeight: 650, color: "var(--tx1)" }}>{t(title)}</div>
            <div style={{ flex: 1 }} />
            <button className="hov" onClick={onClose} title={t("settings:sections.close")}
              style={{ width: 28, height: 28, borderRadius: "50%", border: "none",
                background: "var(--fill4)", color: "var(--tx3b)", cursor: "default",
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
