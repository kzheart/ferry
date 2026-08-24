// Agent 集成:把 Ferry 的能力接到用户电脑上的 coding agent 上。
// 两个分区各管一件事——PATH 里的 ferry 命令、共享技能目录里的 Ferry skill。
// Claude Code 不扫描共享目录,宿主会同时在它的原生目录补受管理的 symlink。
// 页面本身不认识任何路径:目标只用 id 指代,路径由宿主算好带回来。
//
// 反馈全部锚在操作点:每行只有一个 StateButton,它同时是状态显示、进度和结果。
// 引擎状态/socket 路径/共享开关这些只有开发者关心的东西不在这里露出——正常工作
// 时用户不需要知道引擎存在,真出问题时错误会落到页面底部的告警行。
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  cliInstall, cliUninstall, integrationStatus, skillInstall, skillUninstall,
} from "../../platform/desktop/client.js";
import StateButton from "../../shared/ui/StateButton.jsx";
import { Card, GroupTitle, Row } from "./parts.jsx";

const mono = { fontSize: 11, color: "var(--tx5)", marginTop: 3, overflowWrap: "anywhere" };

/** 行内幽灵按钮:承载「这一行的第二个动作」,静止时透明(见 app.css 的 .sb-reveal)。 */
function GhostAction({ label, onClick, disabled }) {
  return (
    <button type="button" className="sb-reveal" onClick={onClick} disabled={disabled}
      style={{ height: 30, padding: "0 10px", flex: "none", borderRadius: 8,
        border: "1px solid transparent", background: "transparent", color: "var(--tx3b)",
        fontSize: 12, fontWeight: 600, fontFamily: "inherit", cursor: "default" }}>
      {label}
    </button>
  );
}

function CliSection({ cli, onInstall, onUninstall, t }) {
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
  // 三种状态各自只有一个「下一步」:没装就装,指向旧引擎就更新,装好了就只剩卸载。
  const state = !cli.installed ? t("settings:integration.cli.stateNotInstalled")
    : outdated ? t("settings:integration.cli.stateOutdatedShort")
      : t("settings:integration.cli.stateInstalled");
  const primary = !cli.installed
    ? { label: t("settings:integration.cli.install"), pending: t("settings:integration.cli.installing"), run: onInstall }
    : outdated
      ? { label: t("settings:integration.cli.update"), pending: t("settings:integration.cli.updating"), run: onInstall }
      : { label: t("settings:integration.cli.uninstall"), pending: t("settings:integration.cli.uninstalling"), run: onUninstall, danger: true };
  return (
    <>
      <Card>
        <Row first className="sb-row" title={t("settings:integration.cli.title")}
          desc={cli.installed
            ? t("settings:integration.cli.descInstalled", { path: cli.link_path })
            : t("settings:integration.cli.descNotInstalled")}>
          {/* 更新态下卸载仍要可达,但它不该和「更新」抢位置 */}
          {outdated && (
            <GhostAction label={t("settings:integration.cli.uninstall")} onClick={onUninstall} />
          )}
          <StateButton tone={outdated ? "warn" : cli.installed ? "ok" : "idle"}
            stateLabel={state} actionLabel={primary.label} pendingLabel={primary.pending}
            danger={primary.danger} disabled={!cli.engine_path} onRun={primary.run} />
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

/** 安装真身只有一个;不读共享目录的 Agent 入口由宿主透明补齐。 */
function SkillRow({ target, bundledVersion, onInstall, onRemove, t }) {
  const updatable = target.installed && !!bundledVersion
    && target.installed_version !== bundledVersion;
  // 装好之后版本号本身就是状态,再写一遍「已安装」是重复的
  const state = !target.installed ? t("settings:integration.skills.stateNotInstalled")
    : target.installed_version
      ? t("settings:integration.skills.stateVersion", { version: target.installed_version })
      : t("settings:integration.skills.stateInstalledUnknown");
  const primary = !target.installed
    ? { label: t("settings:integration.skills.install"), pending: t("settings:integration.skills.installing"), run: onInstall }
    : updatable
      ? { label: t("settings:integration.skills.update"), pending: t("settings:integration.skills.updating"), run: onInstall }
      : { label: t("settings:integration.skills.remove"), pending: t("settings:integration.skills.removing"), run: onRemove, danger: true };
  return (
    <Row first className="sb-row" title={t("settings:integration.skills.rowTitle")} desc={target.path}>
      {updatable && (
        <GhostAction label={t("settings:integration.skills.remove")} onClick={onRemove} />
      )}
      <StateButton tone={updatable ? "warn" : target.installed ? "ok" : "idle"}
        stateLabel={state} actionLabel={primary.label} pendingLabel={primary.pending}
        danger={primary.danger} disabled={!bundledVersion && !target.installed}
        onRun={primary.run} />
    </Row>
  );
}

export default function Integration() {
  const { t } = useTranslation();
  const [status, setStatus] = useState(null);
  const [error, setError] = useState(null);

  const message = value => String(value?.message || value || "");

  const refresh = useCallback(async () => {
    try {
      setStatus(await integrationStatus());
    } catch (e) {
      setError(message(e));
    }
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  // 每个动作跑完都重新拉一次状态:安装结果的真相在磁盘上,不在这次调用的返回值里。
  // 失败时把错误抛回 StateButton(它变成「重试」),同时在页面底部说清原因。
  const run = action => async () => {
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(message(e));
      throw e;
    } finally {
      await refresh();
    }
  };

  const bundled = status?.bundled_version || null;
  const updatable = (status?.skills || []).some(s => s.installed && !!bundled
    && s.installed_version !== bundled);

  return (
    <div>
      <GroupTitle first>{t("settings:integration.cli.groupTitle")}</GroupTitle>
      {status && <CliSection cli={status.cli} t={t}
        onInstall={run(cliInstall)} onUninstall={run(cliUninstall)} />}

      {/* 版本是「这一组共用」的信息,放组标题;每行的按钮只说自己的状态 */}
      <GroupTitle right={updatable && bundled
        ? t("settings:integration.skills.groupUpdatable", { version: bundled })
        : undefined}>
        {t("settings:integration.skills.groupTitle")}</GroupTitle>
      <Card>
        {(status?.skills || []).map(target => (
          <SkillRow key={target.id} target={target} bundledVersion={bundled} t={t}
            onInstall={run(() => skillInstall(target.id))}
            onRemove={run(() => skillUninstall(target.id))} />
        ))}
      </Card>
      <div style={{ ...mono, paddingLeft: 2 }}>
        {t("settings:integration.skills.groupHint")}</div>
      {!bundled && status && (
        <div style={{ ...mono, color: "var(--err-deep)", paddingLeft: 2 }}>
          {t("settings:integration.skills.bundleMissing")}</div>
      )}

      {error && <div style={{ ...mono, color: "var(--err-deep)", paddingLeft: 2, marginTop: 10 }}
        role="alert">{error}</div>}
    </div>
  );
}
