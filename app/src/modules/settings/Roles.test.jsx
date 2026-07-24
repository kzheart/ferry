// 角色页:内置角色和自定义角色走的是同一套表单,区别只在危险区给的是
// "恢复默认"还是"删除"。详情区被拆成了 RoleList / RoleToolGrid / RoleIconPicker,
// 这里守的就是拆分后 props 还接得上——类型检查看不见 JSX 的传递链路。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render, screen } from "@testing-library/react";

import { FerryRuntimeProvider } from "../../shared/capabilities/ferryRuntime.jsx";
import Roles from "./Roles.jsx";

const role = (id, extra = {}) => ({
  id, name: id, description: "", persona: "", tools: ["session_read"],
  allow_bash: false, apply_policy: "manual", builtin: false, ...extra,
});

function mount(overrides = {}) {
  const calls = [];
  const ferry = {
    roles: [role("default", { builtin: true, name: "Ferry" }), role("reader")],
    models: [],
    reloadRoles: async () => {},
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

test("内置角色可以编辑,危险区给的是恢复默认而不是删除", async () => {
  const calls = mount();

  const name = screen.getByDisplayValue("Ferry");
  assert.equal(name.disabled, false);
  assert.ok(screen.getByText("settings:roles.resetTitle"));
  assert.equal(screen.queryByText("settings:roles.dangerTitle"), null);

  // 两步确认:第一下只换文案,第二下才真的落到运行时
  fireEvent.click(screen.getByText("settings:roles.reset"));
  assert.deepEqual(calls, []);
  fireEvent.click(screen.getByText("settings:roles.resetConfirm"));
  await Promise.resolve();
  assert.deepEqual(calls, [["reset", "default"]]);
});

test("自定义角色仍然是删除,工具卡的勾选会写回草稿", () => {
  mount();
  fireEvent.click(screen.getByText("reader"));

  assert.ok(screen.getByText("settings:roles.dangerTitle"));
  assert.equal(screen.queryByText("settings:roles.resetTitle"), null);

  // 未改动时保存按钮是灰的,点一下工具卡就该亮起来
  const save = screen.getByText("settings:roles.save");
  assert.equal(save.disabled, true);
  fireEvent.click(screen.getByText("settings:roles.tool.usage.label"));
  assert.equal(screen.getByText("settings:roles.save").disabled, false);
});
