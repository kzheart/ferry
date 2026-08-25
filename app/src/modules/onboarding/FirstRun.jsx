// 首次启动向导:整窗六站航线——欢迎 → 外观与语言 → 检测到的工具 → Agent 集成 →
// 续聊与迁移 → 扫描与索引。进度指示是一条流动的虚线航线,小船随步骤开往下一站。
//
// 终点站分两个阶段:先读取会话元数据(短,读完列表即可用),再建全文索引(长,
// 只影响搜索)。索引期间唯一的主按钮是「进入主界面」,点击弹确认框二选一:
// 不等了直接进入 / 等待索引完成;索引就绪时无论停在哪一步的哪个状态都自动进入。
// 集成与外观步骤都复用设置页的宿主接口与同一份偏好,任何一站都不拦路。
// 动效类(fw-*)与时长曲线定义在 app.css「首启向导」一节。
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  cliInstall, integrationStatus, skillInstall,
} from "../../platform/desktop/client.js";
import { TOOL_NAME, TOOLS } from "../../shared/contracts/tools.js";
import { LOCALE_META } from "../../shared/i18n/index.js";
import { ToolIcon, Spinner } from "../../shared/ui/icons.jsx";
import { ConfirmBox } from "../../shared/ui/ConfirmBox.jsx";
import StateButton from "../../shared/ui/StateButton.jsx";
import appIcon from "../../assets/app-icon.png";
import { useIndexProgress } from "./useIndexProgress.js";

const N_STEPS = 6;
const LAST = N_STEPS - 1;

const rowStyle = {
  display: "flex", alignItems: "center", gap: 11, padding: "10px 12px",
  border: "1px solid var(--line5)", borderRadius: 8, marginBottom: 8,
};
const mono = { fontSize: 11, color: "var(--tx5)", overflowWrap: "anywhere" };
const routeStyle = {
  position: "absolute", height: 2,
  background: "repeating-linear-gradient(90deg, var(--line-strong) 0 5px, transparent 5px 9px)",
};
const stepTitle = { fontSize: 21, fontWeight: 600, letterSpacing: "-.015em" };
const stepDesc = {
  fontSize: 13.5, color: "var(--tx3b)", margin: "8px 0 18px", lineHeight: 1.65, maxWidth: "36em",
};

// 级联入场:包装层带延迟,首条 80ms、逐条 +45ms(声呐行光经 inherit 拿同一延迟)
const cascade = i => ({
  className: "fw-st",
  style: { animationDelay: `${80 + i * 45}ms` },
});

export function BoatGlyph() {
  return (
    <svg width="16" height="13" viewBox="0 0 16 13" fill="currentColor" aria-hidden>
      <path d="M1 8.2h14L12.8 12H3.2L1 8.2Z" />
      <path d="M8.9 1.2 13.2 7H8.9V1.2Z" opacity=".85" />
      <rect x="8.1" y="1" width="1" height="6.2" rx=".5" />
    </svg>
  );
}

/** 无动作可做时的静态状态(圆点 + 文案),和 StateButton 的静止态同构。 */
function StateChip({ tone, label }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 11,
      color: tone === "ok" ? "var(--ok-deep)" : "var(--tx5)", fontWeight: 600, flex: "none" }}>
      <span style={{ width: 7, height: 7, borderRadius: "50%",
        background: tone === "ok" ? "var(--ok)" : "var(--line-strong)" }} />
      {label}
    </span>
  );
}

function WelcomeStep({ t }) {
  const highlights = [
    t("onboarding:wizard.highlightBrowse"),
    t("onboarding:wizard.highlightHandoff"),
    t("onboarding:wizard.highlightUsage"),
  ];
  return (
    <>
      <div style={{ width: 86 }}>
        <img className="noinvert fw-harbor-icon" src={appIcon} alt="Ferry"
          width={52} height={52} style={{ display: "block" }} />
        <div className="fw-wave" style={{ width: 86, height: 8, overflow: "hidden",
          marginTop: 7, opacity: 0.75 }}>
          <svg width="128" height="8" viewBox="0 0 128 8" fill="none" aria-hidden>
            <path d="M0 4 Q4 1 8 4 T16 4 T24 4 T32 4 T40 4 T48 4 T56 4 T64 4 T72 4 T80 4 T88 4 T96 4 T104 4 T112 4 T120 4 T128 4"
              stroke="var(--accent)" strokeOpacity=".5" strokeWidth="1.5" />
          </svg>
        </div>
      </div>
      <div style={{ ...stepTitle, fontSize: 24, marginTop: 18 }}>
        {t("onboarding:welcome.title")}</div>
      <div style={stepDesc}>{t("onboarding:welcome.desc")}</div>
      <div>
        {highlights.map((text, i) => (
          <div key={text} {...cascade(i)}>
            <div style={{ display: "flex", gap: 10, alignItems: "flex-start", padding: "7px 0" }}>
              <span style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--accent)",
                flex: "none", marginTop: 6 }} />
              <span style={{ fontSize: 13.5, color: "var(--tx2)", lineHeight: 1.5 }}>{text}</span>
            </div>
          </div>
        ))}
      </div>
    </>
  );
}

const THEME_PREVIEWS = [
  { id: "light", bg: "#F5F6F8", bar: "#D8DCE3", dot: "#1C2530" },
  { id: "dark", bg: "#16181E", bar: "#2E323B", dot: "#E8E9EB" },
  { id: "system", bg: "linear-gradient(105deg, #F5F6F8 50%, #16181E 50%)", bar: "#9AA1AD", dot: "#5A6472" },
];

function AppearanceStep({ prefs, onPrefs, t }) {
  const theme = prefs?.theme || "light";
  const locale = prefs?.locale ?? "";
  return (
    <>
      <div style={stepTitle}>{t("onboarding:wizard.appearanceTitle")}</div>
      <div style={stepDesc}>{t("onboarding:wizard.appearanceDesc")}</div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10, marginBottom: 16 }}>
        {THEME_PREVIEWS.map((m, i) => {
          const sel = theme === m.id;
          return (
            <div key={m.id} {...cascade(i)}>
              <button type="button" onClick={() => onPrefs?.({ theme: m.id })}
                aria-pressed={sel}
                style={{ width: "100%", padding: 6, borderRadius: 10, cursor: "default",
                  background: "none", fontFamily: "inherit", textAlign: "center",
                  border: sel ? "1px solid var(--accent)" : "1px solid var(--line5)",
                  boxShadow: sel ? "0 0 0 1px var(--accent)" : "none",
                  transition: "border-color .18s, box-shadow .18s" }}>
                <span className="noinvert" style={{ display: "block", height: 60, borderRadius: 6,
                  position: "relative", overflow: "hidden", border: "1px solid var(--line5)",
                  background: m.bg }}>
                  <i style={{ position: "absolute", left: 8, right: 8, top: 11, height: 5,
                    borderRadius: 3, background: m.bar, opacity: 0.9 }} />
                  <i style={{ position: "absolute", left: 8, width: "46%", top: 24, height: 5,
                    borderRadius: 3, background: m.bar, opacity: 0.6 }} />
                  <i style={{ position: "absolute", left: 8, width: "26%", top: 37, height: 5,
                    borderRadius: 3, background: m.dot }} />
                  {sel && (
                    <span style={{ position: "absolute", right: 5, bottom: 5, width: 16, height: 16,
                      borderRadius: "50%", background: "var(--accent)", color: "var(--accent-fg)",
                      fontSize: 10, lineHeight: "16px", fontWeight: 700,
                      animation: "fsettle .34s cubic-bezier(.34,1.4,.64,1)" }}>✓</span>
                  )}
                </span>
                <span style={{ display: "block", fontSize: 12, fontWeight: 600, marginTop: 6,
                  color: sel ? "var(--accent)" : "var(--tx3b)" }}>
                  {t(`settings:theme.${m.id}`)}</span>
              </button>
            </div>
          );
        })}
      </div>
      <div {...cascade(3)}>
        <div style={{ display: "flex", gap: 8 }}>
          {[["", t("common:language.followSystem")],
            ...LOCALE_META.map(l => [l.code, l.nativeName])].map(([value, label]) => {
            const sel = locale === value;
            return (
              <button key={value || "system"} type="button" aria-pressed={sel}
                onClick={() => onPrefs?.({ locale: value || null })}
                style={{ height: 30, padding: "0 14px", borderRadius: 99, fontSize: 12,
                  fontWeight: 600, fontFamily: "inherit", cursor: "default",
                  border: sel ? "1px solid var(--accent)" : "1px solid var(--line6)",
                  background: sel ? "var(--fill4)" : "none",
                  color: sel ? "var(--accent)" : "var(--tx3b)", transition: "all .18s" }}>
                {label}
              </button>
            );
          })}
        </div>
      </div>
    </>
  );
}

function ToolsStep({ env, scan, t }) {
  return (
    <>
      <div style={stepTitle}>{t("onboarding:welcome.detectedTools")}</div>
      <div style={stepDesc}>{t("onboarding:wizard.toolsDesc")}</div>
      <div style={{ position: "relative", overflow: "hidden", borderRadius: 10 }}>
        {TOOLS.map((tool, i) => {
          const info = env?.[tool] || {};
          const found = scan?.tools?.[tool];
          const installed = info.installed;
          const detect = found?.ok
            ? t("onboarding:welcome.detectWithSessions", { path: found.path, count: found.count })
            : installed ? t("onboarding:welcome.detectInstalled")
              : t("onboarding:welcome.detectNotFound");
          return (
            <div key={tool} {...cascade(i)}>
              <div className="fw-rowlit" style={rowStyle}>
                <ToolIcon tool={tool} size={26}
                  dot={installed ? "var(--ok)" : "var(--line-strong)"} />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 13, color: "var(--tx2)", fontWeight: 600 }}>{TOOL_NAME[tool]}</div>
                  <div style={{ fontSize: 11, color: "var(--tx4)" }}>{detect}</div>
                </div>
                <StateChip tone={installed ? "ok" : "idle"}
                  label={installed ? t("onboarding:welcome.badgeInstalled")
                    : t("onboarding:welcome.badgeNotInstalled")} />
              </div>
            </div>
          );
        })}
        <i className="fw-sweep" style={{ position: "absolute", left: -4, right: -4, top: 0,
          height: 44, pointerEvents: "none",
          background: "linear-gradient(180deg, transparent, color-mix(in srgb, var(--accent) 14%, transparent), transparent)" }} />
      </div>
    </>
  );
}

function IntegrationStep({ t }) {
  const [status, setStatus] = useState(null);
  const [error, setError] = useState(null);
  // 安装成功的行泛一层绿色涟漪:计数器换 key 重挂行节点,动画就重放一次
  const [flashed, setFlashed] = useState({});

  const message = value => String(value?.message || value || "");

  const refresh = useCallback(async () => {
    try {
      setStatus(await integrationStatus());
    } catch (e) {
      setError(message(e));
    }
  }, []);

  // 浏览器开发预览里没有 Tauri 宿主,不拉状态也不报错,只留说明文字
  const desktop = typeof window !== "undefined" && !!window.__TAURI_INTERNALS__;
  useEffect(() => { if (desktop) refresh(); }, [desktop, refresh]);

  // 与设置页同款:动作跑完回读磁盘,失败把按钮变「重试」并在底部说明原因
  const run = (id, action) => async () => {
    setError(null);
    try {
      await action();
      setFlashed(v => ({ ...v, [id]: (v[id] || 0) + 1 }));
    } catch (e) {
      setError(message(e));
      throw e;
    } finally {
      await refresh();
    }
  };

  const cli = status?.cli;
  const bundled = status?.bundled_version || null;
  const cliOutdated = cli?.installed && !cli.points_to_current_engine;

  return (
    <>
      <div style={stepTitle}>{t("onboarding:wizard.integrationTitle")}</div>
      <div style={stepDesc}>{t("onboarding:wizard.integrationDesc")}</div>

      {cli && (
        <div {...cascade(0)}>
          <div key={`cli-${flashed.cli || 0}`} style={rowStyle}
            className={flashed.cli ? "fw-flash" : undefined}>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 13, color: "var(--tx2)", fontWeight: 600 }}>
                {t("settings:integration.cli.title")}</div>
              <div style={{ fontSize: 11, color: "var(--tx4)", overflowWrap: "anywhere" }}>
                {!cli.supported
                  ? (cli.unsupported_reason || t("settings:integration.cli.unsupported"))
                  : cli.installed
                    ? t("settings:integration.cli.descInstalled", { path: cli.link_path })
                    : t("settings:integration.cli.descNotInstalled")}
              </div>
            </div>
            {cli.supported && (cli.installed && !cliOutdated
              ? <StateChip tone="ok" label={t("settings:integration.cli.stateInstalled")} />
              : (
                <StateButton tone={cliOutdated ? "warn" : "idle"}
                  stateLabel={cliOutdated
                    ? t("settings:integration.cli.stateOutdatedShort")
                    : t("settings:integration.cli.stateNotInstalled")}
                  actionLabel={cliOutdated
                    ? t("settings:integration.cli.update")
                    : t("settings:integration.cli.install")}
                  pendingLabel={cliOutdated
                    ? t("settings:integration.cli.updating")
                    : t("settings:integration.cli.installing")}
                  disabled={!cli.engine_path} onRun={run("cli", cliInstall)} />
              ))}
          </div>
        </div>
      )}

      {(status?.skills || []).map((target, i) => {
        const updatable = target.installed && !!bundled
          && target.installed_version !== bundled;
        return (
          <div key={target.id} {...cascade(i + 1)}>
            <div key={`${target.id}-${flashed[target.id] || 0}`} style={rowStyle}
              className={flashed[target.id] ? "fw-flash" : undefined}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, color: "var(--tx2)", fontWeight: 600 }}>
                  {t("settings:integration.skills.rowTitle")}</div>
                <div style={{ fontSize: 11, color: "var(--tx4)", overflowWrap: "anywhere" }}>
                  {target.path}</div>
              </div>
              {target.installed && !updatable
                ? <StateChip tone="ok" label={target.installed_version
                  ? t("settings:integration.skills.stateVersion", { version: target.installed_version })
                  : t("settings:integration.skills.stateInstalledUnknown")} />
                : (
                  <StateButton tone={updatable ? "warn" : "idle"}
                    stateLabel={updatable
                      ? t("settings:integration.skills.stateVersion", { version: target.installed_version })
                      : t("settings:integration.skills.stateNotInstalled")}
                    actionLabel={updatable
                      ? t("settings:integration.skills.update")
                      : t("settings:integration.skills.install")}
                    pendingLabel={updatable
                      ? t("settings:integration.skills.updating")
                      : t("settings:integration.skills.installing")}
                    disabled={!bundled && !target.installed}
                    onRun={run(target.id, () => skillInstall(target.id))} />
                )}
            </div>
          </div>
        );
      })}

      <div style={{ ...mono, paddingLeft: 2, marginTop: 2 }}>
        {t("onboarding:wizard.integrationHint")}</div>
      {error && <div style={{ ...mono, color: "var(--err-deep)", paddingLeft: 2, marginTop: 8 }}
        role="alert">{error}</div>}
    </>
  );
}

// 渡运演示的两个码头:真实的 Agent 图标 + 名字。左码头是货源(Claude Code),
// 会话条的颜色与它一致;右码头(Codex)在船靠岸的节拍上微微一涨。
function Dock({ tool, side, arrive }) {
  return (
    <span className={arrive ? "fw-arrive" : undefined}
      style={{ position: "absolute", [side]: 30, top: 26, display: "inline-flex",
        flexDirection: "column", alignItems: "center", gap: 5 }}>
      <ToolIcon tool={tool} size={34} />
      <span style={{ fontSize: 10, fontWeight: 600, color: "var(--tx4)", whiteSpace: "nowrap" }}>
        {TOOL_NAME[tool]}</span>
    </span>
  );
}

function HandoffStep({ t }) {
  return (
    <>
      <div style={stepTitle}>{t("onboarding:wizard.handoffTitle")}</div>
      <div style={{ ...stepDesc, marginBottom: 14 }}>{t("onboarding:wizard.handoffDesc")}</div>
      <div {...cascade(0)}>
        <div style={{ position: "relative", height: 108, border: "1px solid var(--line5)",
          borderRadius: 10, background: "var(--inset)", overflow: "hidden", marginBottom: 14 }}
          aria-hidden>
          <Dock tool="claude" side="left" />
          <Dock tool="codex" side="right" arrive />
          <i className="fw-route" style={{ ...routeStyle, left: 80, right: 80, top: 62 }} />
          <span className="fw-cross" style={{ position: "absolute", left: 76, top: 42,
            color: "var(--accent)" }}>
            <span style={{ position: "absolute", left: 2, top: -5, width: 11, height: 4,
              borderRadius: 2, background: "#D97757" }} />
            <BoatGlyph />
          </span>
        </div>
      </div>
      {[t("onboarding:wizard.handoffResume"), t("onboarding:wizard.handoffMigrate")].map((text, i) => (
        <div key={text} {...cascade(i + 1)}>
          <div style={{ display: "flex", gap: 10, alignItems: "flex-start", padding: "6px 0" }}>
            <span style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--accent)",
              flex: "none", marginTop: 6 }} />
            <span style={{ fontSize: 13.5, color: "var(--tx2)", lineHeight: 1.55 }}>{text}</span>
          </div>
        </div>
      ))}
    </>
  );
}

// 终点站:扫描与索引。阶段一读会话元数据(分工具进度),阶段二建全文索引
// (百分比 + 剩余时间)。索引就绪时上层自动进入,这里只负责展示。
function ScanStep({ metaDone, progress, ci, t }) {
  // 收尾阶段:文件读完了,引擎在算身份摘要/整理索引(首启冷扫描时是大头)。
  // 六条已满的工具条收起来,换成一条有终点的整理进度,避免「条满了还转圈」的假死感。
  if (!metaDone && progress?.state === "running" && progress?.phase === "finalizing") {
    const fin = progress.finalizing || {};
    const fTotal = fin.total || 0;
    const fDone = Math.min(fin.processed || 0, fTotal);
    const fPct = fTotal > 0 ? Math.min(100, Math.round(fDone / fTotal * 100)) : 0;
    return (
      <>
        <div style={stepTitle}>{t("onboarding:wizard.scanTitleFinalizing")}</div>
        <div style={{ ...stepDesc, marginBottom: 6 }}>{t("onboarding:wizard.scanDescFinalizing")}</div>
        <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12, fontWeight: 600,
          color: "var(--ok-deep)", margin: "10px 0 4px" }}>
          ✓ {t("onboarding:wizard.scanReadDone", { count: progress.total ?? 0 })}
        </div>
        {fTotal > 0 ? (
          <>
            <div style={{ height: 6, borderRadius: 99, background: "var(--inset)",
              overflow: "hidden", margin: "14px 0 10px" }}>
              <i style={{ display: "block", height: "100%", borderRadius: 99,
                width: `${fPct}%`, background: "var(--accent)", transition: "width .3s" }} />
            </div>
            <div style={{ fontSize: 12, color: "var(--tx4)", fontVariantNumeric: "tabular-nums" }}>
              {t("onboarding:wizard.finalizingFrac", { done: fDone, total: fTotal })}
            </div>
          </>
        ) : (
          <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12,
            fontWeight: 600, color: "var(--tx4)", marginTop: 8 }}>
            <Spinner size={12} accent="var(--accent)" />
            {t("onboarding:wizard.finalizingHint")}
          </div>
        )}
      </>
    );
  }
  if (!metaDone) {
    const tools = progress?.state === "running" ? Object.entries(progress.tools || {}) : [];
    return (
      <>
        <div style={stepTitle}>{t("onboarding:wizard.scanTitleReading")}</div>
        <div style={stepDesc}>{t("onboarding:wizard.scanDescReading")}</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 7, marginBottom: 14 }}>
          {tools.map(([tool, p]) => {
            const total = p.total ?? null;
            const pct = p.done ? 100 : total ? Math.min(100, p.processed / total * 100) : 0;
            return (
              <div key={tool} style={{ display: "flex", alignItems: "center", gap: 10,
                fontSize: 12, color: "var(--tx3b)" }}>
                <span style={{ width: 96, fontWeight: 600, color: "var(--tx2)" }}>
                  {TOOL_NAME[tool] || tool}</span>
                <span style={{ flex: 1, height: 4, borderRadius: 99, background: "var(--inset)",
                  overflow: "hidden" }}>
                  <i style={{ display: "block", height: "100%", borderRadius: 99,
                    width: `${pct}%`, background: p.done ? "var(--ok)" : "var(--accent)",
                    transition: "width .3s" }} />
                </span>
                <span className="num" style={{ width: 90, textAlign: "right", color: "var(--tx4)",
                  fontVariantNumeric: "tabular-nums" }}>
                  {p.done ? "✓" : total ? `${p.processed} / ${total}` : p.processed}</span>
              </div>
            );
          })}
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12,
          fontWeight: 600, color: "var(--tx4)" }}>
          <Spinner size={12} accent="var(--accent)" />
          {t("onboarding:wizard.scanReadingHint")}
        </div>
      </>
    );
  }

  const total = ci ? (ci.indexed_sessions || 0) + (ci.pending_sessions || 0) : 0;
  // 未就绪时封顶 99:索引一就绪上层就自动进入了,这里永远不该显示 100%。
  const pct = ci && total > 0 ? Math.min(99, Math.floor(ci.indexed_sessions / total * 100)) : 0;
  return (
    <>
      <div style={stepTitle}>{t("onboarding:wizard.scanTitleIndexing")}</div>
      <div style={{ ...stepDesc, marginBottom: 6 }}>{t("onboarding:wizard.scanDescIndexing")}</div>
      {(total > 0 || (progress?.total ?? 0) > 0) && (
        <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12, fontWeight: 600,
          color: "var(--ok-deep)", margin: "10px 0 4px" }}>
          ✓ {t("onboarding:wizard.scanReadDone", { count: total || (progress?.total ?? 0) })}
        </div>
      )}
      {ci ? (
        <>
          <div style={{ display: "flex", alignItems: "baseline", gap: 10 }}>
            <span style={{ fontSize: 40, fontWeight: 650, letterSpacing: "-.02em",
              fontVariantNumeric: "tabular-nums" }}>{pct}%</span>
            <span style={{ fontSize: 13, color: "var(--tx4)", fontVariantNumeric: "tabular-nums" }}>
              {t("onboarding:wizard.indexedFrac", { indexed: ci.indexed_sessions, total })}</span>
          </div>
          <div style={{ height: 6, borderRadius: 99, background: "var(--inset)",
            overflow: "hidden", margin: "14px 0 10px" }}>
            <i style={{ display: "block", height: "100%", borderRadius: 99,
              width: `${pct}%`, background: "var(--accent)", transition: "width .5s linear" }} />
          </div>
          <div style={{ fontSize: 12, color: "var(--tx4)" }}>
            {t("onboarding:wizard.waitingHint")}
          </div>
        </>
      ) : (
        <div style={{ fontSize: 12, color: "var(--tx4)", marginTop: 8 }}>
          {t("onboarding:wizard.scanUnavailable")}</div>
      )}
    </>
  );
}

// 航线进度:虚线流动,站点随进度点亮,小船开往当前站。
// 挂在换页动画容器之外,DOM 节点跨步骤存活,left 过渡才能生效。
function Voyage({ step }) {
  return (
    <div style={{ position: "relative", width: 190, height: 20 }} aria-hidden>
      <i className="fw-route" style={{ ...routeStyle, left: 2, right: 2, top: 13 }} />
      {[...Array(N_STEPS)].map((_, i) => (
        <span key={i} style={{ position: "absolute", top: 11, width: 6, height: 6,
          borderRadius: "50%", left: `calc(${i} / ${N_STEPS - 1} * (100% - 6px))`,
          background: i <= step ? "var(--accent)" : "var(--line-strong)",
          boxShadow: "0 0 0 3px var(--bg)", transition: "background .3s" }} />
      ))}
      <span className="fw-boat" style={{ position: "absolute", top: -2, color: "var(--accent)",
        left: `calc(${step} / ${N_STEPS - 1} * (100% - 16px))` }}>
        <BoatGlyph />
      </span>
    </div>
  );
}

export default function FirstRun({ env, scan, prefs, onPrefs, onScan, scanning, onStart }) {
  const { t } = useTranslation();
  const [step, setStep] = useState(0);
  const [dir, setDir] = useState(0);
  const [confirming, setConfirming] = useState(false);
  const scanStarted = useRef(false);
  const sawScanning = useRef(false);
  const entered = useRef(false);
  const last = step === LAST;

  const { progress, contentIndex: ci } =
    useIndexProgress({ active: last, interval: 600 });

  // 走到终点站才开扫;向导重看时若已扫过,doScan 的幂等守卫会兜住
  useEffect(() => {
    if (last && !scanStarted.current) {
      scanStarted.current = true;
      onScan?.();
    }
  }, [last, onScan]);
  useEffect(() => { if (scanning) sawScanning.current = true; }, [scanning]);

  // 「读取完成」以引擎为准:scan_progress 不在 running 且覆盖度已有数字,
  // 说明引擎手里有一份完整的会话快照——不管扫描是谁触发的(前端、启动预热)。
  // 只信前端自己那次 engine("scan") 的生死是不可靠的:首启冷扫描期间串行池
  // 可能被引擎内部工作占住,调用迟迟不返回,但进度条(引擎侧)早就读满了。
  // 拿不到任何引擎进度(浏览器开发预览)才退回看本地 scanning 的起落。
  const coverageKnown = !!ci && typeof ci.indexed_sessions === "number";
  const metaDone = progress != null
    ? progress.state !== "running" && coverageKnown
    : scanStarted.current && sawScanning.current && !scanning;
  const indexReady = !!ci?.ready;

  const enter = useCallback(() => {
    if (entered.current) return;
    entered.current = true;
    onStart();
  }, [onStart]);

  // 元数据读完且索引就绪就直接进——覆盖「等待完成」和「会话很少秒扫完」两种情况
  useEffect(() => {
    if (last && metaDone && indexReady) enter();
  }, [last, metaDone, indexReady, enter]);

  const goStep = next => {
    setDir(next > step ? 1 : -1);
    setStep(next);
  };

  const onPrimary = () => {
    if (!last) { goStep(step + 1); return; }
    // 索引没就绪(或拿不到进度)都先确认一次;就绪时上面的效应已经自动进入了
    if (indexReady) enter();
    else setConfirming(true);
  };

  const totalSessions = ci ? (ci.indexed_sessions || 0) + (ci.pending_sessions || 0) : 0;

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0,
      position: "relative", background: "var(--bg)" }}>
      <div style={{ flex: 1, minHeight: 0, overflowY: "auto", display: "flex",
        alignItems: "center", justifyContent: "center", padding: "24px 48px 8px" }}>
        <div style={{ width: 560, maxWidth: "100%" }}>
          <div key={step} className={dir > 0 ? "fw-fwd" : dir < 0 ? "fw-back" : undefined}>
            {step === 0 && <WelcomeStep t={t} />}
            {step === 1 && <AppearanceStep prefs={prefs} onPrefs={onPrefs} t={t} />}
            {step === 2 && <ToolsStep env={env} scan={scan} t={t} />}
            {step === 3 && <IntegrationStep t={t} />}
            {step === 4 && <HandoffStep t={t} />}
            {step === 5 && <ScanStep metaDone={metaDone} progress={progress} ci={ci} t={t} />}
          </div>
        </div>
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "14px 48px 24px" }}>
        <div style={{ flex: 1, display: "flex", justifyContent: "center" }}>
          <div style={{ width: 560, maxWidth: "100%", display: "flex", alignItems: "center" }}>
            <div style={{ flex: 1 }}><Voyage step={step} /></div>
            {step > 0 && (
              <button className="fbtn" onClick={() => goStep(step - 1)}
                style={{ height: 34, padding: "0 16px", borderRadius: 8, fontSize: 13,
                  marginRight: 8 }}>
                {t("onboarding:guide.back")}</button>
            )}
            <button className="fbtn-primary" onClick={onPrimary}
              disabled={last && !metaDone}
              style={{ height: 34, padding: "0 18px", borderRadius: 8, fontSize: 13,
                opacity: last && !metaDone ? 0.55 : 1 }}>
              {last ? t("onboarding:wizard.enterNow") : t("onboarding:wizard.next")}</button>
          </div>
        </div>
      </div>

      {confirming && (
        <ConfirmBox width={420} title={t("onboarding:wizard.confirmTitle")}
          actions={
            <>
              <button className="fbtn" style={{ height: 32, padding: "0 14px", fontSize: 13 }}
                onClick={() => setConfirming(false)}>
                {t("onboarding:wizard.confirmWait")}</button>
              <button className="fbtn-primary" style={{ height: 32, padding: "0 14px", fontSize: 13 }}
                onClick={enter}>
                {t("onboarding:wizard.confirmEnter")}</button>
            </>
          }>
          <div style={{ fontSize: 13, color: "var(--tx3b)", marginTop: 8, lineHeight: 1.65 }}>
            {ci
              ? t("onboarding:wizard.confirmBody", {
                indexed: ci.indexed_sessions, total: totalSessions,
              })
              : t("onboarding:wizard.confirmBodyNoProgress")}
          </div>
        </ConfirmBox>
      )}
    </div>
  );
}
