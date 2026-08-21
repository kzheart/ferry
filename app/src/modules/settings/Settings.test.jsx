// 设置页与特性开关的关系:分区表按表项标的 feature 过滤、停在被隐藏分区上要回落、
// 「实验性功能」分区由契约驱动渲染。断言绑在文案 key 上(cimode 回显),不看排版。
import { beforeEach, expect, test, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";

import { FerryRuntimeProvider } from "../../shared/capabilities/ferryRuntime.jsx";
import SettingsPage from "./Settings.jsx";

const calls = { get: 0, set: [] };
let hostAgent = false;
let setFails = null;

// 只替换特性开关那两条,其余传输层保持真实:设置页里还挂着别的分区。
vi.mock("../../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  featuresList: async () => {
    calls.get += 1;
    return [{
      id: "builtin-agent", stage: "experimental", default: false,
      enabled: hostAgent,
    }];
  },
  featureSet: async (id, enabled) => {
    calls.set.push([id, enabled]);
    if (setFails) throw new Error(setFails);
    hostAgent = enabled;
  },
}));

const noop = () => {};

const updater = {
  phase: "idle", currentVersion: "0.7.0", update: null, downloaded: 0, total: null,
  error: null, failedAction: null, supported: true, checkForUpdate: noop,
  downloadUpdate: noop, installAndRestart: noop,
};

const settings = {
  theme: "system", locale: null, runtimeProbe: false, terminalApp: "auto",
  reduceMotion: false, sessionOptimization: false, autoCheckUpdates: true,
};

beforeEach(() => {
  calls.get = 0;
  calls.set = [];
  hostAgent = false;
  setFails = null;
});

// 真实树里设置页挂在 FerryRuntimeProvider 之内(助手分区要读它),这里照搬。
const ferry = {
  available: true, activeId: null, activeLog: null, sessions: [], roles: [],
  models: [], mode: "auto", health: null, lastError: null,
  selectedRoleId: "default", clearError: noop,
};

const mount = async (props = {}) => {
  const view = render(
    <FerryRuntimeProvider value={ferry}>
      <SettingsPage settings={settings} setSettings={noop} scan={null} env={{}}
        scanning={false} onRescan={noop} updater={updater} guideSeen onOpenGuide={noop}
        onFirstRun={noop} onClose={noop} {...props} />
    </FerryRuntimeProvider>,
  );
  // 挂载期要回读一次宿主的开关,断言前先让它落地
  await act(async () => {});
  return view;
};

const railLabels = () =>
  [...document.querySelectorAll("button")]
    .map((button) => button.textContent.trim())
    .filter((label) => label.startsWith("settings:sections."));

const clickText = async (text) => {
  const node = [...document.querySelectorAll("button")]
    .find((button) => button.textContent.trim() === text);
  expect(node, `找不到按钮：${text}`).toBeTruthy();
  await act(async () => { node.click(); });
};

const AGENT_TABS = [
  "settings:sections.providers", "settings:sections.models",
  "settings:sections.roles", "settings:sections.skills",
];

test("标了 feature 的四个分区在开关关着时不出现,没标的照旧", async () => {
  await mount();
  const labels = railLabels();
  for (const tab of AGENT_TABS) expect(labels).not.toContain(tab);
  for (const tab of ["settings:sections.prefs", "settings:sections.integration",
    "settings:sections.sources", "settings:sections.updates",
    "settings:sections.experimental"]) {
    expect(labels).toContain(tab);
  }
});

test("助手开着时四个助手分区回来", async () => {
  hostAgent = true;
  await mount();
  const labels = railLabels();
  for (const tab of AGENT_TABS) expect(labels).toContain(tab);
});

test("被隐藏分区不能作为入口停留:回落到偏好设置", async () => {
  await mount({ initialSection: "roles" });
  // 标题区渲染的是回落之后的分区
  expect(screen.getAllByText("settings:sections.prefs").length).toBeGreaterThan(1);
  expect(screen.queryByText("settings:theme.label")).toBeTruthy();
});

// 「实验性功能」分区里那一行的开关。文案 key 由契约的 id 推出来,分区自己不写
// 任何一个具体特性的名字。
const featureToggle = (id) =>
  screen.getByText(`settings:features.${id}.desc`)
    .parentElement.parentElement.querySelector("button");

test("实验分区由契约驱动:stage 为 experimental 的条目逐条渲染,初值来自宿主", async () => {
  await mount();
  await clickText("settings:sections.experimental");
  expect(calls.get).toBe(1);
  // 标题与说明按 id 约定取,分区自己不写任何一个具体特性的名字
  expect(screen.getByText("settings:features.builtin-agent.title")).toBeTruthy();
  expect(featureToggle("builtin-agent").getAttribute("aria-pressed")).toBe("false");
  // 生效时机是分区级的一条规则,不逐个特性抄
  expect(screen.getByText("settings:experimental.togglesNote")).toBeTruthy();
});

test("打开开关:写宿主,四个助手分区当场出现,无需重启", async () => {
  await mount();
  await clickText("settings:sections.experimental");
  const toggle = featureToggle("builtin-agent");

  await act(async () => { toggle.click(); });

  expect(calls.set).toEqual([["builtin-agent", true]]);
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  for (const tab of AGENT_TABS) expect(railLabels()).toContain(tab);
});

test("宿主写入失败时保持原值并报错", async () => {
  setFails = "写入 host-settings.json 失败";
  await mount();
  await clickText("settings:sections.experimental");
  const toggle = featureToggle("builtin-agent");

  await act(async () => { toggle.click(); });

  expect(screen.getByRole("alert").textContent).toContain("写入 host-settings.json 失败");
  expect(toggle.getAttribute("aria-pressed")).toBe("false");
});
