// 角色页:内置角色和自定义角色走的是同一套表单。删除在左栏工具条的减号里,
// 详情区只给内置角色留了"恢复默认"。详情区被拆成了 RoleList / RoleToolGrid /
// RoleIconPicker,这里守的就是拆分后 props 还接得上——类型检查看不见 JSX 的传递链路。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render, screen } from "@testing-library/react";

import { FerryRuntimeProvider } from "../../shared/capabilities/ferryRuntime.jsx";
import Roles from "./Roles.jsx";
import { TOOLS } from "./roleForm.js";
import { TOOL_GLYPH } from "./roleGlyphs.jsx";
import zhSettings from "../../shared/i18n/locales/zh-CN/settings.json";
import enSettings from "../../shared/i18n/locales/en/settings.json";

const role = (id, extra = {}) => ({
  id, name: id, description: "", persona: "", tools: ["session_read"],
  skills: [], apply_policy: "manual", builtin: false, ...extra,
});

function mount(overrides = {}) {
  const calls = [];
  const ferry = {
    roles: [role("default", { builtin: true, name: "Ferry" }), role("reader")],
    models: [],
    reloadRoles: async () => {},
    reloadSkills: async () => {},
    skills: { skills: [], global: [], scan_sources: [] },
    updateRole: async input => calls.push(["update", input]),
    resetRole: async id => calls.push(["reset", id]),
    deleteRole: async id => calls.push(["delete", id]),
    createRole: async () => {},
    copyRole: async () => ({ id: "copy" }),
    ...overrides,
  };
  render(
    <FerryRuntimeProvider value={ferry}>
      <Roles />
    </FerryRuntimeProvider>,
  );
  return calls;
}

const minusButton = () => screen.getByLabelText("settings:roles.delete");

test("内置角色可以编辑,给的是恢复默认,减号禁用", async () => {
  const calls = mount();

  const name = screen.getByDisplayValue("Ferry");
  assert.equal(name.disabled, false);
  assert.ok(screen.getByText("settings:roles.resetTitle"));
  assert.equal(minusButton().disabled, true);

  // 两步确认:第一下只换文案,第二下才真的落到运行时
  fireEvent.click(screen.getByText("settings:roles.reset"));
  assert.deepEqual(calls, []);
  fireEvent.click(screen.getByText("settings:roles.resetConfirm"));
  await Promise.resolve();
  assert.deepEqual(calls, [["reset", "default"]]);
});

test("自定义角色从左栏减号删除,二次确认后才落到运行时", async () => {
  const calls = mount();
  fireEvent.click(screen.getByText("reader"));

  // 详情区不再有删除入口,危险区只留给内置角色的恢复默认
  assert.equal(screen.queryByText("settings:roles.delete"), null);
  assert.equal(screen.queryByText("settings:roles.resetTitle"), null);

  assert.equal(minusButton().disabled, false);
  fireEvent.click(minusButton());
  assert.ok(screen.getByText("settings:roles.deleteAsk"));
  assert.deepEqual(calls, []);

  fireEvent.mouseDown(screen.getByText("settings:roles.delete"));
  await Promise.resolve();
  assert.deepEqual(calls, [["delete", "reader"]]);
});

test("工具卡的勾选会写回草稿", () => {
  mount();
  fireEvent.click(screen.getByText("reader"));

  // 没有改动就没有保存按钮,点一下工具卡才出现——按钮在不在就是脏标记
  assert.equal(screen.queryByText("settings:roles.save"), null);
  fireEvent.click(screen.getByText("settings:roles.tool.usage.label"));
  assert.equal(screen.getByText("settings:roles.save").disabled, false);
  assert.ok(screen.getByLabelText("settings:roles.discard"));
});

test("bash 是能力里的一张卡,安全区不再有占位开关", () => {
  mount();
  assert.ok(screen.getByText("settings:roles.tool.bash.label"));
  assert.equal(screen.queryByText("settings:roles.bashLater"), null);
});

test("agent_prompt 是第七项高权限能力,勾选后可保存到角色", async () => {
  const calls = mount();
  fireEvent.click(screen.getByText("reader"));

  assert.equal(TOOLS.length, 7);
  assert.ok(TOOL_GLYPH.agent_prompt);
  assert.equal(zhSettings.roles.tool.agent_prompt.label, "驱动 Agent");
  assert.equal(enSettings.roles.tool.agent_prompt.label, "Drive Agent");
  assert.match(zhSettings.roles.tool.agent_prompt.desc, /修改工作区或会话/);
  assert.match(enSettings.roles.tool.agent_prompt.desc, /modify the workspace or session/);

  fireEvent.click(screen.getByText("settings:roles.tool.agent_prompt.label"));
  fireEvent.click(screen.getByText("settings:roles.save"));
  await Promise.resolve();

  assert.equal(calls[0][0], "update");
  assert.ok(calls[0][1].tools.includes("agent_prompt"));
});

// 草稿是在渲染期跟着 baseline 换的,不是等 effect——否则切角色会闪一帧上一个角色的
// 脏状态。这里守的是同一条路径的另一头:选中项没变、内容被外部改写(恢复出厂设置)也要跟上。
test("角色内容被外部改写后,详情区跟着更新", () => {
  const ferry = {
    roles: [role("default", { builtin: true, name: "Ferry" }), role("reader")],
    models: [],
    reloadRoles: async () => {},
    reloadSkills: async () => {},
    skills: { skills: [], global: [], scan_sources: [] },
    updateRole: async () => {}, resetRole: async () => {}, deleteRole: async () => {},
    createRole: async () => {}, copyRole: async () => ({ id: "copy" }),
  };
  const tree = value => (
    <FerryRuntimeProvider value={value}><Roles /></FerryRuntimeProvider>);
  const { rerender } = render(tree(ferry));
  fireEvent.click(screen.getByText("reader"));
  assert.ok(screen.getByDisplayValue("reader"));

  rerender(tree({
    ...ferry,
    roles: [ferry.roles[0], role("reader", { name: "reader 已重置" })],
  }));
  assert.ok(screen.getByDisplayValue("reader 已重置"));
  assert.equal(screen.queryByText("settings:roles.save"), null);
});

test("技能库为空时角色页给空态而不是一片空白", () => {
  mount();
  fireEvent.click(screen.getByText("reader"));
  assert.ok(screen.getByText("settings:skills.roleEmpty"));
});

test("勾选一个已导入技能写回草稿,保存按钮出现", () => {
  mount({
    skills: {
      skills: [{ id: "code-review", name: "代码评审", description: "", broken: false }],
      global: [], scan_sources: [],
    },
  });
  fireEvent.click(screen.getByText("reader"));
  assert.equal(screen.queryByText("settings:roles.save"), null);
  fireEvent.click(screen.getByText("代码评审"));
  assert.equal(screen.getByText("settings:roles.save").disabled, false);
});

test("通用技能在角色层勾选态锁定,不可取消", () => {
  mount({
    skills: {
      skills: [{ id: "code-review", name: "代码评审", description: "", broken: false }],
      global: ["code-review"], scan_sources: [],
    },
  });
  fireEvent.click(screen.getByText("reader"));
  assert.ok(screen.getByText("settings:skills.globalBadge"));
  fireEvent.click(screen.getByText("代码评审"));
  // 锁死的行点不动,草稿不脏就不会冒出保存按钮
  assert.equal(screen.queryByText("settings:roles.save"), null);
});
