// 设置悬浮弹窗(参考 LM Studio):左侧分类 + 偏好设置 / 数据来源
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { TOOL_NAME, TOOLS } from "../../shared/contracts/tools.js";
import { LOCALE_META } from "../../shared/i18n/index.js";
import { useScanProgress } from "../browser/public.js";
import { RefreshIcon, SetGlyph, Spinner, TerminalIcon, ToolIcon } from "../../shared/ui/icons.jsx";
import { Card, GroupTitle, Row, Segmented, Select, Toggle } from "./parts.jsx";
import { useDensity, writeDensity } from "../../shared/ui/density.js";
import Providers from "./Providers.jsx";
import Models from "./Models.jsx";
import Roles from "./Roles.jsx";
import Skills from "./Skills.jsx";
import Integration from "./Integration.jsx";
import Experimental from "./Experimental.jsx";
import {
  filterByFeatures,
  useFeaturesList,
  useIsFeatureEnabled,
} from "../../shared/capabilities/features.jsx";

// 分区表。标了 feature 的分区跟着开关走——providers/models/roles/skills 四项只属于
// 内置 AI 助手;「Agent 集成」不在其列:那是把 Ferry 接到用户自己的 coding agent
// 上的引擎侧功能,与内置助手无关。
const SECTIONS = [
  { key: "prefs", labelKey: "settings:sections.prefs" },
  { key: "providers", labelKey: "settings:sections.providers", feature: "builtin-agent" },
  { key: "models", labelKey: "settings:sections.models", feature: "builtin-agent" },
  { key: "roles", labelKey: "settings:sections.roles", feature: "builtin-agent" },
  { key: "skills", labelKey: "settings:sections.skills", feature: "builtin-agent" },
  { key: "integration", labelKey: "settings:sections.integration" },
  { key: "sources", labelKey: "settings:sections.sources" },
  { key: "updates", labelKey: "settings:sections.updates" },
  { key: "experimental", labelKey: "settings:sections.experimental" },
];

function TerminalAppIcon({ app, size = 16 }) {
  if (app === "terminal") return <TerminalIcon size={size} />;
  if (app === "iterm") return (
    <svg viewBox="0 0 16 16" width={size} height={size} aria-hidden style={{ flex: "none" }}>
      <rect x="1.35" y="1.35" width="13.3" height="13.3" rx="3.1" fill="#202A37" />
      <path d="m4.2 5 2.35 3-2.35 3M8.7 11h3" fill="none" stroke="#69D88E" strokeWidth="1.5"
        strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
  if (app === "warp") return (
    <svg viewBox="0 0 16 16" width={size} height={size} aria-hidden style={{ flex: "none" }}>
      <rect x="1.35" y="1.35" width="13.3" height="13.3" rx="3.1" fill="#FA6B3A" />
      <path d="M8 3.4c2.7 0 4.7 2.3 4.1 4.9-.5 2-2.3 3.4-4.3 3.3-1.7-.1-3-1.6-2.7-3.3.2-1.2 1.2-2 2.4-1.9 1 .1 1.7.9 1.5 1.9"
        fill="none" stroke="white" strokeWidth="1.35" strokeLinecap="round" />
    </svg>
  );
  return (
    <svg viewBox="0 0 16 16" width={size} height={size} aria-hidden style={{ flex: "none" }}>
      <rect x="1.65" y="1.65" width="5.35" height="5.35" rx="1.3" fill="var(--tx4)" />
      <rect x="9" y="1.65" width="5.35" height="5.35" rx="1.3" fill="var(--tx4)" opacity=".72" />
      <rect x="1.65" y="9" width="5.35" height="5.35" rx="1.3" fill="var(--tx4)" opacity=".72" />
      <rect x="9" y="9" width="5.35" height="5.35" rx="1.3" fill="var(--tx4)" opacity=".46" />
    </svg>
  );
}

function TerminalPicker({ value, onChange, t }) {
  const [open, setOpen] = useState(false);
  const [menuPos, setMenuPos] = useState(null);
  const rootRef = useRef(null);
  const menuRef = useRef(null);
  const options = [
    ["auto", t("settings:terminal.auto")],
    ["terminal", t("settings:terminal.terminal")],
    ["iterm", t("settings:terminal.iterm")],
    ["warp", t("settings:terminal.warp")],
  ];
  const current = options.find(([key]) => key === value) || options[0];

  useEffect(() => {
    if (!open) return undefined;
    const position = () => {
      const rect = rootRef.current?.getBoundingClientRect();
      if (rect) setMenuPos({ top: rect.bottom + 6, left: rect.right - 194 });
    };
    const close = event => {
      if (!rootRef.current?.contains(event.target) && !menuRef.current?.contains(event.target)) setOpen(false);
    };
    const escape = event => { if (event.key === "Escape") setOpen(false); };
    position();
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", escape);
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", escape);
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
    };
  }, [open]);

  return (
    <div ref={rootRef} style={{ position: "relative", flex: "none" }}>
      <button type="button" onClick={() => setOpen(v => !v)} aria-haspopup="listbox" aria-expanded={open}
        style={{ minWidth: 168, height: 32, padding: "0 10px", borderRadius: 9,
          border: `1px solid ${open ? "var(--accent)" : "var(--line4)"}`,
          background: "var(--surface)", color: "var(--tx1)", display: "flex", alignItems: "center",
          gap: 8, fontSize: 12, fontWeight: 600, fontFamily: "inherit", cursor: "default",
          boxShadow: open ? "0 0 0 2px var(--acc-soft3)" : "none" }}>
        <TerminalAppIcon app={current[0]} />
        <span style={{ flex: 1, textAlign: "left" }}>{current[1]}</span>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden
          style={{ color: "var(--tx4)", transform: open ? "rotate(180deg)" : "none", transition: "transform .15s ease" }}>
          <path d="M2 4l3 3 3-3" fill="none" stroke="currentColor" strokeWidth="1.6"
            strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {open && menuPos && createPortal(
        <div ref={menuRef} role="listbox" aria-label={t("settings:terminal.app")}
          style={{ position: "fixed", top: menuPos.top, left: menuPos.left, zIndex: 70, minWidth: 194, padding: 5,
            border: "1px solid var(--line3)", borderRadius: 11, background: "var(--surface)",
            boxShadow: "0 14px 28px rgba(0,0,0,.20)" }}>
          {options.map(([key, label]) => {
            const selected = key === current[0];
            return (
              <button key={key} type="button" role="option" aria-selected={selected}
                onClick={() => { onChange(key); setOpen(false); }}
                style={{ width: "100%", height: 32, padding: "0 8px", border: "none", borderRadius: 7,
                  background: selected ? "var(--acc-soft5)" : "transparent", color: "var(--tx1)",
                  display: "flex", alignItems: "center", gap: 9, textAlign: "left", fontFamily: "inherit",
                  fontSize: 12, fontWeight: selected ? 650 : 550, cursor: "default" }}>
                <TerminalAppIcon app={key} />
                <span style={{ flex: 1 }}>{label}</span>
                {selected && <span style={{ color: "var(--accent)", fontSize: 15, lineHeight: 1 }}>✓</span>}
              </button>
            );
          })}
        </div>, document.body
      )}
    </div>
  );
}

function Prefs({ s, set, guideSeen, onOpenGuide }) {
  const { t } = useTranslation();
  const localeValue = s.locale ?? "";
  // 密度不进 ferry-settings:它要在 React 挂载之前就落到根节点上(见 main.jsx),
  // 单独一个键读起来最省事
  const density = useDensity();
  return (
    <div style={{  }}>
      <GroupTitle first>{t("settings:theme.groupTitle")}</GroupTitle>
      <Card>
        <Row first title={t("settings:theme.label")}>
          <Select value={s.theme} onChange={theme => set({ theme })}>
            <option value="light">{t("settings:theme.light")}</option>
            <option value="dark">{t("settings:theme.dark")}</option>
            <option value="system">{t("settings:theme.system")}</option>
          </Select>
        </Row>
      </Card>

      <GroupTitle>{t("settings:density.groupTitle")}</GroupTitle>
      <Card>
        <Row first title={t("settings:density.label")} desc={t("settings:density.desc")}>
          <Segmented value={density} label={t("settings:density.label")}
            options={[["compact", t("settings:density.compact")],
              ["standard", t("settings:density.standard")]]}
            onChange={writeDensity} />
        </Row>
      </Card>

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

      <GroupTitle>{t("settings:terminal.groupTitle")}</GroupTitle>
      <Card>
        <Row first title={t("settings:terminal.app")} desc={t("settings:terminal.appDesc")}>
          <TerminalPicker value={s.terminalApp} onChange={v => set({ terminalApp: v })} t={t} />
        </Row>
      </Card>

      <GroupTitle>{t("settings:guideSection.groupTitle")}</GroupTitle>
      <Card>
        <Row first title={t("settings:guideSection.guide")} desc={t("settings:guideSection.guideDesc")}>
          <button className="fbtn" style={{ height: 30, padding: "0 13px", fontSize: 12 }}
            onClick={onOpenGuide}>{guideSeen ? t("settings:guideSection.reviewGuide") : t("settings:guideSection.quickStart")}</button>
        </Row>
      </Card>
    </div>
  );
}

function Sources({ scan, env, scanning, onRescan }) {
  const { t } = useTranslation();
  const scanProgress = useScanProgress(scanning);
  const tools = scan?.tools || {};
  const connected = TOOLS.filter(t2 => tools[t2]?.ok).length;
  const total = TOOLS.reduce((a, t2) => a + (tools[t2]?.count || 0), 0);
  const indexing = scanning && scanProgress?.total > 0;
  return (
    <div style={{  }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, margin: "0 0 9px 2px" }}>
        <div style={{ flex: 1, fontSize: 11, fontWeight: 600, color: "var(--tx5)",
          letterSpacing: ".05em" }}>{t("settings:sources.connectedTools")}</div>
        <div style={{ fontSize: 11, color: "var(--tx4)" }}>
          {t("settings:sources.connectedMeta", { connected, total })}</div>
        <button className="ftool-btn" style={{ flex: "none" }}
          title={scanning ? t("settings:sources.scanning") : t("settings:sources.rescan")}
          onClick={onRescan} disabled={scanning}>
          {scanning ? <Spinner size={14} /> : <RefreshIcon size={14} />}
        </button>
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
            </div>
          );
        })}
      </Card>
      {indexing && (
        <div style={{ marginTop: 12, padding: "0 2px" }}>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between",
            fontSize: 11, color: "var(--accent)", marginBottom: 6,
            fontVariantNumeric: "tabular-nums" }}>
            <span>{scanProgress.phase === "finalizing"
              ? t("settings:sources.finalizing")
              : t("settings:sources.indexing", {
                done: scanProgress.processed, total: scanProgress.total,
              })}</span>
            <span>{Math.min(100,
              Math.round(scanProgress.processed / scanProgress.total * 100))}%</span>
          </div>
          <div style={{ height: 4, borderRadius: 2, background: "var(--line5)",
            overflow: "hidden" }}>
            <div style={{ height: "100%", background: "var(--accent)",
              width: `${Math.min(100, scanProgress.processed / scanProgress.total * 100)}%`,
              transition: "width .3s ease" }} />
          </div>
        </div>
      )}
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
  installing: "settings:updates.phase.installing",
  error: "settings:updates.phase.error",
};

function Updates({ s, set, updater }) {
  const { t, i18n } = useTranslation();
  const { phase, currentVersion, update, error, failedAction, supported,
    checkForUpdate, startUpdate } = updater;
  const busy = phase === "checking" || phase === "downloading" || phase === "installing";
  const canCheck = supported && !busy;

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
        {/* 下载进度不在这里重复:导航栏「设置」行的环形图标已经在说同一件事。
            这一屏留给侧栏图标塞不进去的东西——失败原因与重试。 */}
        <div aria-live="polite" aria-busy={busy} style={{ padding: 16 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
            <div style={{ flex: "1 1 240px", minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: error ? "var(--err-deep)" : "var(--tx1)" }}>
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
              {(phase === "available" || (phase === "error" && failedAction === "update")) &&
                <button className="fbtn-primary" onClick={startUpdate}
                  style={{ height: 30, padding: "0 13px" }}>{failedAction === "update" ? t("settings:updates.retryUpdate") : t("settings:updates.update")}</button>}
            </div>
          </div>
        </div>
      </Card>
      <div style={{ fontSize: 11, color: "var(--tx5)", marginTop: 10, lineHeight: 1.55, paddingLeft: 2 }}>
        {t("settings:updates.footnote")}</div>
    </div>
  );
}

export default function SettingsPage({ settings, setSettings, scan, env, scanning,
  onRescan, updater, guideSeen, onOpenGuide, onClose, initialSection }) {
  const { t } = useTranslation();
  const features = useFeaturesList();
  const isFeatureEnabled = useIsFeatureEnabled();
  const sections = useMemo(
    () => filterByFeatures(SECTIONS, isFeatureEnabled),
    [isFeatureEnabled],
  );
  const [chosen, setSection] = useState(initialSection || "prefs");
  // 停在一个刚被隐藏的分区上(比如就在这一页把助手关掉)时回落到偏好设置
  const section = sections.some(({ key }) => key === chosen) ? chosen : "prefs";
  const title = sections.find(({ key }) => key === section)?.labelKey;

  return (
    <div onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}
      style={{ position: "absolute", inset: 0, zIndex: 60, display: "flex", alignItems: "center",
        justifyContent: "center", background: "var(--scrim)" }}>
      {/* 980 是角色页详情区("身份/人设/能力/模型/安全"分组 + 双列工具卡)不挤的下限 */}
      <div className="settings-sheet" style={{ width: "min(980px, calc(100vw - 40px))", height: "min(648px, calc(100vh - 48px))",
        display: "flex", borderRadius: 14, overflow: "hidden", background: "var(--settings-bg)",
        border: "1px solid var(--line)", boxShadow: "var(--shadow-sheet)",
         }}>
        <div style={{ width: 196, flex: "none", background: "var(--settings-rail)",
          borderRight: "1px solid var(--line)", display: "flex", flexDirection: "column",
          padding: "16px 12px" }}>
          <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx5)", letterSpacing: ".08em",
            padding: "2px 8px 12px" }}>{t("settings:sections.railTitle")}</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            {sections.map(({ key: k, labelKey }) => {
              const on = section === k;
              return (
                // 侧栏底色本身就是浅灰(--settings-rail),hov-item 压不出对比,用 hov-rail
                <button key={k} className={on ? undefined : "hov-rail"} onClick={() => setSection(k)}
                  style={{ display: "flex", alignItems: "center", gap: 11, height: 36, padding: "0 11px",
                    border: "none", borderRadius: 8, background: on ? "var(--seg-on)" : "transparent",
                    color: on ? "var(--tx1)" : "var(--tx2b)", fontSize: 13, fontWeight: on ? 650 : 500,
                    cursor: "default", textAlign: "left", transition: "background .12s ease" }}>
                  <SetGlyph name={k} color={on ? "var(--tx1)" : "var(--tx3b)"} />{t(labelKey)}
                </button>
              );
            })}
          </div>
        </div>

        <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
          <div style={{ height: 54, flex: "none", display: "flex", alignItems: "center", gap: 12,
            padding: "0 20px", borderBottom: "1px solid var(--line4)" }}>
            <div style={{ fontSize: 15, fontWeight: 600, color: "var(--tx1)" }}>{t(title)}</div>
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
          {section === "providers" ? (
            <Providers />
          ) : section === "models" ? (
            <Models onOpenProviders={() => setSection("providers")} />
          ) : section === "roles" ? (
            <Roles />
          ) : section === "skills" ? (
            <Skills />
          ) : (
            <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "20px 24px" }}>
              <div style={{ maxWidth: 620, margin: "0 auto" }}>
                {section === "prefs" && <Prefs s={settings} set={setSettings} guideSeen={guideSeen}
                  onOpenGuide={onOpenGuide} />}
                {section === "integration" && <Integration />}
                {section === "experimental" && <Experimental features={features} />}
                {section === "sources" && <Sources scan={scan} env={env}
                  scanning={scanning} onRescan={onRescan} />}
                {section === "updates" && <Updates s={settings} set={setSettings} updater={updater} />}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
