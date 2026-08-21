// Agent 集成:把 Ferry 的能力接到用户电脑上的 coding agent 上。
// 三个分区各管一件事——PATH 里的 ferry 命令、各 agent skill 目录里的 Ferry skill、
// 引擎服务状态。页面本身不认识任何路径:目标只用 id 指代,路径由宿主算好带回来。
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  cliInstall, cliUninstall, engineDaemonStop, engineServiceStatus, getEngineShare,
  integrationStatus, pickSkillDirectory, setEngineShare, skillInstall, skillInstallCustom,
  skillUninstall,
} from "../../platform/desktop/client.js";
import { Card, GroupTitle, Row, Toggle } from "./parts.jsx";

const mono = { fontSize: 11, color: "var(--tx5)", marginTop: 3, overflowWrap: "anywhere" };

function ActionButton({ label, onClick, disabled, primary }) {
  return (
    <button className={primary ? "fbtn-primary" : "fbtn"} onClick={onClick} disabled={disabled}
      style={{ height: 30, padding: "0 13px", fontSize: 12, flex: "none" }}>{label}</button>
  );
}

/** 状态点 + 文案,与「数据来源」分区同一套视觉语言。 */
function StateBadge({ ok, warn, text }) {
  const color = ok ? "var(--ok)" : warn ? "var(--warn)" : "var(--tx5)";
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 5, fontSize: 11,
      fontWeight: 600, color: "var(--tx3b)" }}>
      <span style={{ width: 6, height: 6, borderRadius: "50%", background: color }} />{text}
    </span>
  );
}

function CliSection({ cli, busy, onInstall, onUninstall, t }) {
  if (!cli.supported) {
    return (
      <Card>
        <Row first title={t("settings:integration.cli.title")}
          desc={cli.unsupported_reason || t("settings:integration.cli.unsupported")} />
      </Card>
    );
  }
  const linkDir = cli.link_path?.replace(/\/[^/]*$/, "") || "";
  const outdated = cli.installed && !cli.points_to_current_engine;
  return (
    <>
      <Card>
        <Row first title={t("settings:integration.cli.title")}
          desc={cli.installed
            ? t("settings:integration.cli.descInstalled", { path: cli.link_path })
            : t("settings:integration.cli.descNotInstalled")}>
          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <StateBadge ok={cli.installed && !outdated} warn={outdated}
              text={!cli.installed ? t("settings:integration.cli.stateNotInstalled")
                : outdated ? t("settings:integration.cli.stateOutdated")
                  : t("settings:integration.cli.stateInstalled")} />
            <ActionButton primary={!cli.installed || outdated} disabled={!!busy || !cli.engine_path}
              label={cli.installed ? t("settings:integration.cli.update") : t("settings:integration.cli.install")}
              onClick={onInstall} />
            {cli.installed && <ActionButton disabled={!!busy}
              label={t("settings:integration.cli.uninstall")} onClick={onUninstall} />}
          </div>
        </Row>
      </Card>
      {cli.installed && cli.link_target && (
        <div style={{ ...mono, paddingLeft: 2 }} className="mono">
          {t("settings:integration.cli.linkTarget", { path: cli.link_target })}</div>
      )}
      {!cli.engine_path && (
        <div style={{ ...mono, color: "var(--err-deep)", paddingLeft: 2 }}>
          {t("settings:integration.cli.engineMissing")}</div>
      )}
      {cli.installed && !cli.on_path && (
        <div style={{ ...mono, paddingLeft: 2 }}>
          {t("settings:integration.cli.pathHint", { dir: linkDir })}</div>
      )}
    </>
  );
}

function SkillRow({ target, bundledVersion, busy, onInstall, onRemove, t, first }) {
  const name = target.display_name || t("settings:integration.skills.sharedTarget");
  const updatable = target.installed && !!bundledVersion
    && target.installed_version !== bundledVersion;
  const state = !target.installed ? t("settings:integration.skills.stateNotInstalled")
    : target.via_shared ? t("settings:integration.skills.viaShared")
      : updatable ? t("settings:integration.skills.stateUpdatable", { version: bundledVersion })
        : target.installed_version
          ? t("settings:integration.skills.stateInstalled", { version: target.installed_version })
          : t("settings:integration.skills.stateInstalledUnknown");
  return (
    <Row first={first} title={name} desc={target.path}>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <StateBadge ok={target.installed && !updatable} warn={updatable} text={state} />
        {/* 经共享仓库生效的行不给安装:再装一次会把 symlink 换成实体目录,反而断了共享 */}
        {!target.via_shared && (
          <ActionButton primary={!target.installed || updatable}
            disabled={!!busy || !bundledVersion}
            label={target.installed ? t("settings:integration.skills.update")
              : t("settings:integration.skills.install")}
            onClick={onInstall} />
        )}
        {target.installed && <ActionButton disabled={!!busy}
          label={t("settings:integration.skills.remove")} onClick={onRemove} />}
      </div>
    </Row>
  );
}

function EngineSection({ service, share, busy, onShare, onStop, t }) {
  const state = service?.state || "stopped";
  const text = state === "app-shared" ? t("settings:integration.engine.stateAppShared")
    : state === "daemon" ? t("settings:integration.engine.stateDaemon", { pid: service.pid })
      : t("settings:integration.engine.stateStopped");
  const socket = !service?.socket ? t("settings:integration.engine.socketNone")
    : service.socket_ready ? service.socket
      : t("settings:integration.engine.socketMissing", { path: service.socket });
  return (
    <Card>
      <Row first title={t("settings:integration.engine.state")}>
        <StateBadge ok={state !== "stopped"} text={text} />
      </Row>
      <Row title={t("settings:integration.engine.socket")}>
        <span className="mono" style={{ fontSize: 11, color: "var(--tx3b)",
          overflowWrap: "anywhere" }}>{socket}</span>
      </Row>
      {service?.version && (
        <Row title={t("settings:integration.engine.version")}>
          <span className="mono" style={{ fontSize: 12, color: "var(--tx3b)" }}>v{service.version}</span>
        </Row>
      )}
      {/* 开关只写宿主的配置文件,不动正在跑的引擎:sidecar 只在启动时决定要不要监听 */}
      <Row title={t("settings:integration.engine.share")}
        desc={t("settings:integration.engine.shareDesc")}>
        <span style={{ opacity: busy ? 0.45 : 1, pointerEvents: busy ? "none" : undefined,
          flex: "none" }}>
          <Toggle on={share} onChange={onShare} />
        </span>
      </Row>
      {/* 只有 CLI 拉起的 daemon 能从这里停;App 自己的引擎由引擎侧结构化拒绝 */}
      <Row title={t("settings:integration.engine.stop")}
        desc={t("settings:integration.engine.stopDesc")}>
        <ActionButton disabled={!!busy || state !== "daemon"}
          label={t("settings:integration.engine.stop")} onClick={onStop} />
      </Row>
    </Card>
  );
}

export default function Integration() {
  const { t } = useTranslation();
  const [status, setStatus] = useState(null);
  const [service, setService] = useState(null);
  // 共享开关的真值在宿主的配置文件里,这里只是它的一份显示副本。
  const [share, setShare] = useState(true);
  const [busy, setBusy] = useState(null);
  const [error, setError] = useState(null);
  const [notice, setNotice] = useState(null);

  const message = value => String(value?.message || value || "");

  const refresh = useCallback(async () => {
    try {
      const [next, engine, sharing] = await Promise.all([
        integrationStatus(), engineServiceStatus(), getEngineShare(),
      ]);
      setStatus(next);
      setService(engine);
      setShare(sharing);
    } catch (e) {
      setError(message(e));
    }
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  // 每个动作跑完都重新拉一次状态:安装结果的真相在磁盘上,不在这次调用的返回值里。
  const run = (key, action) => async () => {
    setBusy(key);
    setError(null);
    setNotice(null);
    try {
      await action();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  // 自定义目录不在固定目标表里,装完之后状态列表看不到它,只能当场回报落点。
  const installCustom = run("custom", async () => {
    const path = await pickSkillDirectory();
    if (!path) return;
    const installed = await skillInstallCustom(path);
    setNotice(t("settings:integration.skills.customDone", { path: installed }));
  });

  // 改完立刻回读(refresh),提示只说「下次启动生效」:spawn 只在 App 启动时发生。
  const toggleShare = next => run("engine-share", async () => {
    await setEngineShare(next);
    setNotice(t("settings:integration.engine.shareRestart"));
  })();

  const stopDaemon = run("engine-stop", async () => {
    try {
      await engineDaemonStop();
      setNotice(t("settings:integration.engine.stopDone"));
    } catch (e) {
      // 宿主给的是 {code, message};只有 app_mode 需要换成自己的解释。
      throw new Error(e?.code === "app_mode"
        ? t("settings:integration.engine.stopAppMode") : message(e));
    }
  });

  const bundled = status?.bundled_version || null;

  return (
    <div>
      <GroupTitle first>{t("settings:integration.cli.groupTitle")}</GroupTitle>
      {status && <CliSection cli={status.cli} busy={busy} t={t}
        onInstall={run("cli-install", cliInstall)}
        onUninstall={run("cli-uninstall", cliUninstall)} />}

      <GroupTitle right={bundled ? t("settings:integration.skills.groupDesc", { version: bundled }) : undefined}>
        {t("settings:integration.skills.groupTitle")}</GroupTitle>
      <Card>
        {(status?.skills || []).map((target, index) => (
          <SkillRow key={target.id} target={target} bundledVersion={bundled} busy={busy} t={t}
            first={index === 0}
            onInstall={run(`skill-install:${target.id}`, () => skillInstall(target.id))}
            onRemove={run(`skill-remove:${target.id}`, () => skillUninstall(target.id))} />
        ))}
      </Card>
      {!bundled && status && (
        <div style={{ ...mono, color: "var(--err-deep)", paddingLeft: 2 }}>
          {t("settings:integration.skills.bundleMissing")}</div>
      )}
      <div style={{ marginTop: 10, paddingLeft: 2 }}>
        <ActionButton disabled={!!busy || !bundled} onClick={installCustom}
          label={t("settings:integration.skills.custom")} />
      </div>

      <GroupTitle>{t("settings:integration.engine.groupTitle")}</GroupTitle>
      <EngineSection service={service} share={share} busy={busy} t={t}
        onShare={toggleShare} onStop={stopDaemon} />

      {notice && <div style={{ ...mono, color: "var(--ok-deep)", paddingLeft: 2, marginTop: 10 }}
        role="status">{notice}</div>}
      {error && <div style={{ ...mono, color: "var(--err-deep)", paddingLeft: 2, marginTop: 10 }}
        role="alert">{error}</div>}
    </div>
  );
}
