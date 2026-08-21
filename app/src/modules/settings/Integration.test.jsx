import { beforeEach, expect, test, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import Integration from "./Integration.jsx";

const calls = {
  cliInstall: 0, cliUninstall: 0, skillInstall: [], skillUninstall: [],
  skillInstallCustom: [], pick: 0, status: 0, setShare: [], stop: 0,
};
let status;
let service;
let share;
let failNext = null;
/** 宿主的结构化失败:{code, message},不是 Error。 */
let stopError = null;

vi.mock("../../platform/desktop/client.js", () => ({
  integrationStatus: async () => { calls.status += 1; return status; },
  engineServiceStatus: async () => service,
  cliInstall: async () => { calls.cliInstall += 1; if (failNext) throw new Error(failNext); },
  cliUninstall: async () => { calls.cliUninstall += 1; },
  skillInstall: async (id) => { calls.skillInstall.push(id); },
  skillUninstall: async (id) => { calls.skillUninstall.push(id); },
  skillInstallCustom: async (path) => { calls.skillInstallCustom.push(path); return `${path}/ferry`; },
  pickSkillDirectory: async () => { calls.pick += 1; return "/tmp/custom-skills"; },
  getEngineShare: async () => share,
  setEngineShare: async (enabled) => { calls.setShare.push(enabled); share = enabled; },
  engineDaemonStop: async () => { calls.stop += 1; if (stopError) throw stopError; },
}));

const CLI_INSTALLED = {
  supported: true, unsupported_reason: null, link_path: "/home/u/.local/bin/ferry",
  installed: true, link_target: "/Apps/Ferry.app/ferry-engine",
  points_to_current_engine: true, engine_path: "/Apps/Ferry.app/ferry-engine", on_path: true,
};

const baseStatus = () => ({
  cli: { ...CLI_INSTALLED },
  bundled_version: "0.7.0",
  skills: [
    { id: "claude", display_name: "Claude Code", path: "/home/u/.claude/skills",
      installed: true, installed_version: "0.7.0", via_shared: false },
    { id: "codex", display_name: "Codex CLI", path: "/home/u/.codex/skills",
      installed: false, installed_version: null, via_shared: false },
    { id: "shared", display_name: "", path: "/home/u/.agents/skills",
      installed: false, installed_version: null, via_shared: false },
  ],
});

beforeEach(() => {
  Object.assign(calls, {
    cliInstall: 0, cliUninstall: 0, skillInstall: [], skillUninstall: [],
    skillInstallCustom: [], pick: 0, status: 0, setShare: [], stop: 0,
  });
  failNext = null;
  stopError = null;
  share = true;
  status = baseStatus();
  service = { state: "stopped", pid: null, socket: null, socket_ready: false, version: null };
});

const mount = async () => {
  const view = render(<Integration />);
  // 首次 effect 里的两条状态查询都要落地,否则断言看到的是空壳
  await act(async () => {});
  return view;
};

const clickText = async (text) => {
  const node = [...document.querySelectorAll("button")]
    .find((button) => button.textContent.trim() === text);
  expect(node, `找不到按钮：${text}`).toBeTruthy();
  await act(async () => { node.click(); });
};

/** Row 的结构是 行 > 文本列 > 标题,从标题往上两级才回到整行。 */
const rowOf = (rowTitle) => screen.getByText(rowTitle).parentElement.parentElement;

/** 一行里的按钮:先按行标题定位到 Row,再在行内找按钮。 */
const clickInRow = async (rowTitle, label) => {
  const node = [...rowOf(rowTitle).querySelectorAll("button")]
    .find((button) => button.textContent.trim() === label);
  expect(node, `${rowTitle} 行里找不到按钮：${label}`).toBeTruthy();
  await act(async () => { node.click(); });
};

test("已安装的 CLI 显示安装点与更新/卸载", async () => {
  await mount();
  expect(screen.getByText("settings:integration.cli.descInstalled")).toBeTruthy();
  expect(screen.getByText("settings:integration.cli.stateInstalled")).toBeTruthy();
  expect(screen.getByText("settings:integration.cli.update")).toBeTruthy();
  expect(screen.getByText("settings:integration.cli.uninstall")).toBeTruthy();
});

test("未安装时只给安装按钮", async () => {
  status.cli = { ...CLI_INSTALLED, installed: false, link_target: null,
    points_to_current_engine: false };
  await mount();
  expect(screen.getByText("settings:integration.cli.stateNotInstalled")).toBeTruthy();
  expect(screen.getByText("settings:integration.cli.install")).toBeTruthy();
  expect(screen.queryByText("settings:integration.cli.uninstall")).toBeNull();
});

test("指向旧引擎时提示需要更新", async () => {
  status.cli = { ...CLI_INSTALLED, points_to_current_engine: false,
    link_target: "/Old/Ferry.app/ferry-engine" };
  await mount();
  expect(screen.getByText("settings:integration.cli.stateOutdated")).toBeTruthy();
});

test("装了但不在 PATH 时给出 shell 配置提示,状态仍算已安装", async () => {
  status.cli = { ...CLI_INSTALLED, on_path: false };
  await mount();
  expect(screen.getByText("settings:integration.cli.stateInstalled")).toBeTruthy();
  expect(screen.getByText("settings:integration.cli.pathHint")).toBeTruthy();
});

test("平台不支持时只展示宿主给的结构化原因", async () => {
  status.cli = { supported: false, unsupported_reason: "Windows 命令行工具安装尚未实现",
    link_path: null, installed: false, link_target: null,
    points_to_current_engine: false, engine_path: null, on_path: false };
  await mount();
  expect(screen.getByText("Windows 命令行工具安装尚未实现")).toBeTruthy();
  expect(screen.queryByText("settings:integration.cli.install")).toBeNull();
});

test("点安装后调用宿主并重新拉状态", async () => {
  status.cli = { ...CLI_INSTALLED, installed: false, link_target: null,
    points_to_current_engine: false };
  await mount();
  expect(calls.status).toBe(1);
  await clickText("settings:integration.cli.install");
  expect(calls.cliInstall).toBe(1);
  expect(calls.status).toBe(2);
});

test("宿主报错时展示错误文本", async () => {
  status.cli = { ...CLI_INSTALLED, installed: false, link_target: null,
    points_to_current_engine: false };
  failNext = "创建 CLI 入口失败: Permission denied";
  await mount();
  await clickText("settings:integration.cli.install");
  expect(screen.getByRole("alert").textContent).toContain("Permission denied");
});

test("skill 目标逐行渲染,共享仓库用兜底名字", async () => {
  await mount();
  expect(screen.getByText("Claude Code")).toBeTruthy();
  expect(screen.getByText("Codex CLI")).toBeTruthy();
  expect(screen.getByText("settings:integration.skills.sharedTarget")).toBeTruthy();
  expect(screen.getByText("/home/u/.claude/skills")).toBeTruthy();
});

test("skill 安装与移除只传 target id", async () => {
  await mount();
  await clickInRow("Codex CLI", "settings:integration.skills.install");
  expect(calls.skillInstall).toEqual(["codex"]);
  await clickInRow("Claude Code", "settings:integration.skills.remove");
  expect(calls.skillUninstall).toEqual(["claude"]);
});

test("已装但版本落后的目标给更新按钮", async () => {
  status.skills[0].installed_version = "0.6.0";
  await mount();
  expect(screen.getByText("settings:integration.skills.stateUpdatable")).toBeTruthy();
  await clickInRow("Claude Code", "settings:integration.skills.update");
  expect(calls.skillInstall).toEqual(["claude"]);
});

test("经共享仓库生效的目标不给安装按钮", async () => {
  status.skills[0] = { ...status.skills[0], via_shared: true };
  await mount();
  expect(screen.getByText("settings:integration.skills.viaShared")).toBeTruthy();
  const labels = [...rowOf("Claude Code").querySelectorAll("button")]
    .map((button) => button.textContent.trim());
  expect(labels).not.toContain("settings:integration.skills.install");
  expect(labels).not.toContain("settings:integration.skills.update");
  expect(labels).toContain("settings:integration.skills.remove");
});

test("自定义目录安装串起目录对话框,并回报落点", async () => {
  await mount();
  await clickText("settings:integration.skills.custom");
  expect(calls.pick).toBe(1);
  expect(calls.skillInstallCustom).toEqual(["/tmp/custom-skills"]);
  expect(screen.getByRole("status").textContent)
    .toContain("settings:integration.skills.customDone");
});

test("打包资源缺失时禁用全部安装入口", async () => {
  status.bundled_version = null;
  await mount();
  expect(screen.getByText("settings:integration.skills.bundleMissing")).toBeTruthy();
  const install = [...document.querySelectorAll("button")]
    .find((button) => button.textContent.trim() === "settings:integration.skills.install");
  expect(install.disabled).toBe(true);
});

test("引擎服务未运行时不显示 pid", async () => {
  await mount();
  expect(screen.getByText("settings:integration.engine.stateStopped")).toBeTruthy();
  expect(screen.getByText("settings:integration.engine.socketNone")).toBeTruthy();
});

test("App 共享中显示 socket 路径与版本", async () => {
  service = { state: "app-shared", pid: 4242, socket: "/home/u/.ferry/engine.sock",
    socket_ready: true, version: "0.7.0" };
  await mount();
  expect(screen.getByText("settings:integration.engine.stateAppShared")).toBeTruthy();
  expect(screen.getByText("/home/u/.ferry/engine.sock")).toBeTruthy();
  expect(screen.getByText("v0.7.0")).toBeTruthy();
});

const daemonRunning = () => ({ state: "daemon", pid: 99, socket: "/home/u/.ferry/engine.sock",
  socket_ready: true, version: "0.7.0" });

// 停止行的标题与按钮同名,按描述文案定位这一行才不会撞上按钮自己。
const STOP_ROW = "settings:integration.engine.stopDesc";
const SHARE_ROW = "settings:integration.engine.shareDesc";
const stopButton = () => rowOf(STOP_ROW).querySelector("button");
const shareToggle = () => rowOf(SHARE_ROW).querySelector("button");

test("未运行时停止按钮不可用", async () => {
  await mount();
  expect(stopButton().disabled).toBe(true);
});

test("独立 daemon 运行时才可以停止", async () => {
  service = daemonRunning();
  await mount();
  expect(screen.getByText("settings:integration.engine.stateDaemon")).toBeTruthy();
  expect(stopButton().disabled).toBe(false);
});

test("App 共享中时停止按钮同样不可用", async () => {
  service = { state: "app-shared", pid: 42, socket: "/home/u/.ferry/engine.sock",
    socket_ready: true, version: "0.7.0" };
  await mount();
  expect(stopButton().disabled).toBe(true);
});

test("停止 daemon 调用宿主并重新拉状态", async () => {
  service = daemonRunning();
  await mount();
  expect(calls.status).toBe(1);
  await clickInRow(STOP_ROW, "settings:integration.engine.stop");
  expect(calls.stop).toBe(1);
  expect(calls.status).toBe(2);
  expect(screen.getByRole("status").textContent)
    .toContain("settings:integration.engine.stopDone");
});

test("App 引擎被拒时给自己的解释而不是宿主原文", async () => {
  service = daemonRunning();
  stopError = { code: "app_mode", message: "这个引擎是 App 自己的 sidecar,不能从这里停止" };
  await mount();
  await clickInRow(STOP_ROW, "settings:integration.engine.stop");
  expect(screen.getByRole("alert").textContent)
    .toContain("settings:integration.engine.stopAppMode");
});

test("其余停止失败原样展示宿主给的说明", async () => {
  service = daemonRunning();
  stopError = { code: "unavailable", message: "无法连接 /home/u/.ferry/engine.sock" };
  await mount();
  await clickInRow(STOP_ROW, "settings:integration.engine.stop");
  expect(screen.getByRole("alert").textContent).toContain("无法连接");
});

test("共享开关初值来自宿主,切换后提示重启生效", async () => {
  share = false;
  await mount();
  expect(shareToggle().getAttribute("aria-pressed")).toBe("false");

  await act(async () => { shareToggle().click(); });
  expect(calls.setShare).toEqual([true]);
  expect(screen.getByRole("status").textContent)
    .toContain("settings:integration.engine.shareRestart");
  // 状态回读之后开关跟着宿主的事实源走
  expect(shareToggle().getAttribute("aria-pressed")).toBe("true");
});
