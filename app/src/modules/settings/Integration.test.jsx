import { beforeEach, expect, test, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import Integration from "./Integration.jsx";

const calls = {
  cliInstall: 0, cliUninstall: 0, skillInstall: [], skillUninstall: [], status: 0,
};
let status;
let failNext = null;

vi.mock("../../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  integrationStatus: async () => { calls.status += 1; return status; },
  cliInstall: async () => { calls.cliInstall += 1; if (failNext) throw new Error(failNext); },
  cliUninstall: async () => { calls.cliUninstall += 1; },
  skillInstall: async (id) => { calls.skillInstall.push(id); },
  skillUninstall: async (id) => { calls.skillUninstall.push(id); },
  featuresList: async () => [{
    id: "handoff", stage: "preferences", default: true, enabled: true,
  }],
  featureSet: async () => {},
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
    { id: "shared", path: "/home/u/.agents/skills",
      installed: true, installed_version: "0.7.0" },
  ],
});

beforeEach(() => {
  Object.assign(calls, {
    cliInstall: 0, cliUninstall: 0, skillInstall: [], skillUninstall: [], status: 0,
  });
  failNext = null;
  status = baseStatus();
});

const mount = async () => {
  const view = render(<Integration />);
  await act(async () => {});
  return view;
};

const buttonWith = (text) => [...document.querySelectorAll("button")]
  .find((button) => button.textContent.trim() === text);

const clickText = async (text) => {
  const node = buttonWith(text);
  expect(node, `找不到按钮：${text}`).toBeTruthy();
  await act(async () => { node.click(); });
};

/** StateButton 静止时显示状态,动作文案要指上去才出现。 */
const hoverText = async (stateText) => {
  const node = buttonWith(stateText);
  expect(node, `找不到状态按钮：${stateText}`).toBeTruthy();
  await act(async () => { fireEvent.mouseEnter(node); });
  return node;
};

test("已安装的 CLI:静止显示已安装,指上去才变成卸载", async () => {
  await mount();
  expect(screen.getByText("settings:integration.cli.descInstalled")).toBeTruthy();
  const node = await hoverText("settings:integration.cli.stateInstalled");
  expect(node.textContent.trim()).toBe("settings:integration.cli.uninstall");
});

test("已安装且是当前引擎时,这一行只有一个控件", async () => {
  await mount();
  const row = screen.getByText("settings:integration.cli.title").parentElement.parentElement;
  expect(row.querySelectorAll("button")).toHaveLength(1);
});

test("未安装时状态为未安装,点一下就装", async () => {
  status.cli = { ...CLI_INSTALLED, installed: false, link_target: null,
    points_to_current_engine: false };
  await mount();
  expect(calls.status).toBe(1);
  await clickText("settings:integration.cli.stateNotInstalled");
  expect(calls.cliInstall).toBe(1);
  // 安装结果的真相在磁盘上,动作跑完必须回读
  expect(calls.status).toBe(2);
});

test("指向旧引擎:状态变旧引擎,主动作是更新,卸载仍然可达", async () => {
  status.cli = { ...CLI_INSTALLED, points_to_current_engine: false,
    link_target: "/Old/Ferry.app/ferry-engine" };
  await mount();
  const node = await hoverText("settings:integration.cli.stateOutdatedShort");
  expect(node.textContent.trim()).toBe("settings:integration.cli.update");
  expect(buttonWith("settings:integration.cli.uninstall")).toBeTruthy();

  await clickText("settings:integration.cli.uninstall");
  expect(calls.cliUninstall).toBe(1);
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
  expect(buttonWith("settings:integration.cli.stateNotInstalled")).toBeFalsy();
});

test("找不到引擎二进制时安装入口不可点", async () => {
  status.cli = { ...CLI_INSTALLED, installed: false, link_target: null,
    points_to_current_engine: false, engine_path: null };
  await mount();
  expect(buttonWith("settings:integration.cli.stateNotInstalled").disabled).toBe(true);
});

test("宿主报错时展示错误文本", async () => {
  status.cli = { ...CLI_INSTALLED, installed: false, link_target: null,
    points_to_current_engine: false };
  failNext = "创建 CLI 入口失败: Permission denied";
  await mount();
  await clickText("settings:integration.cli.stateNotInstalled");
  expect(screen.getByRole("alert").textContent).toContain("Permission denied");
});

test("skill 只有共享技能目录一行,并说清各 agent 共读", async () => {
  await mount();
  expect(screen.getByText("settings:integration.skills.rowTitle")).toBeTruthy();
  expect(screen.getByText("/home/u/.agents/skills")).toBeTruthy();
  expect(screen.getByText("settings:integration.skills.groupHint")).toBeTruthy();
});

test("已装且是最新版:状态显示版本号,主动作是移除", async () => {
  await mount();
  const node = await hoverText("settings:integration.skills.stateVersion");
  expect(node.textContent.trim()).toBe("settings:integration.skills.remove");
  await act(async () => { node.click(); });
  expect(calls.skillUninstall).toEqual(["shared"]);
});

test("未安装时点一下就装,只传 target id", async () => {
  status.skills[0] = { ...status.skills[0], installed: false, installed_version: null };
  await mount();
  await clickText("settings:integration.skills.stateNotInstalled");
  expect(calls.skillInstall).toEqual(["shared"]);
});

test("版本落后时主动作是更新,组标题给出目标版本", async () => {
  status.skills[0].installed_version = "0.6.0";
  await mount();
  expect(screen.getByText("settings:integration.skills.groupUpdatable")).toBeTruthy();
  const node = await hoverText("settings:integration.skills.stateVersion");
  expect(node.textContent.trim()).toBe("settings:integration.skills.update");
  await act(async () => { node.click(); });
  expect(calls.skillInstall).toEqual(["shared"]);
});

test("打包资源缺失时禁用安装入口", async () => {
  status.bundled_version = null;
  status.skills[0] = { ...status.skills[0], installed: false, installed_version: null };
  await mount();
  expect(screen.getByText("settings:integration.skills.bundleMissing")).toBeTruthy();
  expect(buttonWith("settings:integration.skills.stateNotInstalled").disabled).toBe(true);
});

test("引擎状态与共享开关不再出现在这一页", async () => {
  await mount();
  expect(document.body.textContent).not.toContain("integration.engine");
});
