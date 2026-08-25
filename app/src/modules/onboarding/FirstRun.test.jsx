import { beforeEach, expect, test, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import FirstRun from "./FirstRun.jsx";
import { LOCALE_META } from "../../shared/i18n/index.js";

const calls = { cliInstall: 0, skillInstall: [], status: 0, scanProgress: 0 };
let status;
let progressPayload;
let engineDown;

vi.mock("../../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  integrationStatus: async () => { calls.status += 1; return status; },
  cliInstall: async () => { calls.cliInstall += 1; },
  skillInstall: async (id) => { calls.skillInstall.push(id); },
  engine: async (method) => {
    if (engineDown) throw new Error("engine unavailable");
    if (method === "scan_progress") { calls.scanProgress += 1; return progressPayload; }
    throw new Error(`unexpected engine method: ${method}`);
  },
}));

const baseStatus = () => ({
  cli: {
    supported: true, unsupported_reason: null, link_path: "/home/u/.local/bin/ferry",
    installed: false, link_target: null, points_to_current_engine: false,
    engine_path: "/Apps/Ferry.app/ferry-engine", on_path: false,
  },
  bundled_version: "0.8.0",
  skills: [
    { id: "shared", path: "/home/u/.agents/skills", installed: false, installed_version: null },
  ],
});

const indexing = () => ({
  state: "idle", phase: "reading", processed: 2039, total: 2039, tools: {},
  content_index: { ready: false, indexed_sessions: 900, pending_sessions: 1139, building: true },
});

const reading = () => ({
  state: "running", phase: "reading", processed: 300, total: 2039,
  tools: { claude: { processed: 147, total: 147, done: true },
    codex: { processed: 153, total: 1229, done: false } },
  content_index: { ready: false, reason: "not_scanned" },
});

beforeEach(() => {
  Object.assign(calls, { cliInstall: 0, skillInstall: [], status: 0, scanProgress: 0 });
  status = baseStatus();
  progressPayload = indexing();
  engineDown = false;
  // 集成步骤只在桌面宿主里拉状态
  window.__TAURI_INTERNALS__ = {};
});

const buttonWith = (text) => [...document.querySelectorAll("button")]
  .find((button) => button.textContent.trim() === text);

const clickText = async (text) => {
  const node = buttonWith(text);
  expect(node, `找不到按钮：${text}`).toBeTruthy();
  await act(async () => { node.click(); });
};

const next = () => clickText("onboarding:wizard.next");

const defaults = (props = {}) => ({
  env: {}, scan: null, prefs: { theme: "light", locale: null }, onPrefs: () => {},
  onScan: () => {}, scanning: false, onStart: () => {}, ...props,
});

const mount = async (props = {}) => {
  const merged = defaults(props);
  const view = render(<FirstRun {...merged} />);
  await act(async () => {});
  return { view, props: merged };
};

// 直达终点站:引擎侧进度已是「读取完成」(state idle + 覆盖度有数字)
const gotoScanStation = async (props = {}) => {
  const { view } = await mount(props);
  await next(); await next(); await next(); await next(); await next();
  return view;
};

test("第一站是欢迎页:亮点齐全,没有上一步,也不拉集成状态", async () => {
  await mount();
  expect(screen.getByText("onboarding:welcome.title")).toBeTruthy();
  expect(screen.getByText("onboarding:wizard.highlightBrowse")).toBeTruthy();
  expect(buttonWith("onboarding:guide.back")).toBeFalsy();
  expect(calls.status).toBe(0);
  expect(calls.scanProgress).toBe(0);
});

test("第二站外观:点主题卡与语言 pill 都写回同一份偏好", async () => {
  const onPrefs = vi.fn();
  await mount({ onPrefs });
  await next();
  expect(screen.getByText("onboarding:wizard.appearanceTitle")).toBeTruthy();
  await clickText("settings:theme.dark");
  expect(onPrefs).toHaveBeenCalledWith({ theme: "dark" });
  await clickText(LOCALE_META[0].nativeName);
  expect(onPrefs).toHaveBeenCalledWith({ locale: LOCALE_META[0].code });
  // 跟随系统写回 null,不是空串
  await clickText("common:language.followSystem");
  expect(onPrefs).toHaveBeenCalledWith({ locale: null });
});

test("第四站集成:进入才拉状态,CLI 与 skill 都未安装时各给一个安装入口", async () => {
  await mount();
  await next(); await next();
  expect(calls.status).toBe(0);
  await next();
  expect(calls.status).toBe(1);
  await clickText("settings:integration.cli.stateNotInstalled");
  expect(calls.cliInstall).toBe(1);
  // 与设置页同款:动作跑完回读磁盘
  expect(calls.status).toBe(2);
  await clickText("settings:integration.skills.stateNotInstalled");
  expect(calls.skillInstall).toEqual(["shared"]);
});

test("第六站扫描:进入即触发扫描并开始轮询;引擎还在读取时不能进入", async () => {
  progressPayload = reading();
  const onScan = vi.fn();
  await mount({ onScan });
  await next(); await next(); await next(); await next();
  expect(screen.getByText("onboarding:wizard.handoffTitle")).toBeTruthy();
  expect(onScan).not.toHaveBeenCalled();
  await next();
  expect(onScan).toHaveBeenCalledTimes(1);
  expect(calls.scanProgress).toBeGreaterThan(0);
  // 引擎侧还在 running:显示分工具进度,主按钮禁用
  expect(screen.getByText("onboarding:wizard.scanTitleReading")).toBeTruthy();
  expect(buttonWith("onboarding:wizard.enterNow").disabled).toBe(true);
});

test("收尾阶段展示整理进度而不是已满的工具条,主按钮仍禁用", async () => {
  progressPayload = {
    state: "running", phase: "finalizing", processed: 2039, total: 2039,
    tools: { claude: { processed: 2039, total: 2039, done: true } },
    finalizing: { processed: 800, total: 2039 },
    content_index: { ready: false, reason: "not_scanned" },
  };
  await gotoScanStation();
  expect(screen.getByText("onboarding:wizard.scanTitleFinalizing")).toBeTruthy();
  expect(screen.getByText(/scanReadDone/)).toBeTruthy();
  expect(screen.getByText("onboarding:wizard.finalizingFrac")).toBeTruthy();
  expect(screen.queryByText("onboarding:wizard.scanReadingHint")).toBeFalsy();
  expect(buttonWith("onboarding:wizard.enterNow").disabled).toBe(true);
});

test("引擎读取完成后进入索引视图,不依赖前端那次 scan 调用的生死", async () => {
  // scanning 全程 false、从未翻转过——过去的实现在这里会永远卡在读取页
  await gotoScanStation();
  expect(screen.getByText("onboarding:wizard.scanTitleIndexing")).toBeTruthy();
  expect(buttonWith("onboarding:wizard.enterNow").disabled).toBe(false);
});

test("拿不到引擎进度时退回看本地扫描起落,不至于永远禁用", async () => {
  engineDown = true;
  const { view, props } = await mount();
  await next(); await next(); await next(); await next(); await next();
  expect(buttonWith("onboarding:wizard.enterNow").disabled).toBe(true);
  await act(async () => { view.rerender(<FirstRun {...props} scanning={true} />); });
  await act(async () => { view.rerender(<FirstRun {...props} scanning={false} />); });
  expect(buttonWith("onboarding:wizard.enterNow").disabled).toBe(false);
  expect(screen.getByText("onboarding:wizard.scanUnavailable")).toBeTruthy();
});

test("索引未完成时点「进入」弹确认框:可以不等,也可以留下等", async () => {
  let started = 0;
  await gotoScanStation({ onStart: () => { started += 1; } });
  await clickText("onboarding:wizard.enterNow");
  expect(screen.getByText("onboarding:wizard.confirmTitle")).toBeTruthy();
  // 选「等待索引完成」:关掉确认框,留在进度页
  await clickText("onboarding:wizard.confirmWait");
  expect(screen.queryByText("onboarding:wizard.confirmTitle")).toBeFalsy();
  expect(started).toBe(0);
  // 再点一次,这回不等了
  await clickText("onboarding:wizard.enterNow");
  await clickText("onboarding:wizard.confirmEnter");
  expect(started).toBe(1);
});

test("索引就绪时自动进入,不需要任何点击", async () => {
  progressPayload = { ...indexing(),
    content_index: { ready: true, indexed_sessions: 2039, pending_sessions: 0, building: false } };
  let started = 0;
  await gotoScanStation({ onStart: () => { started += 1; } });
  await act(async () => {});
  expect(started).toBe(1);
});
