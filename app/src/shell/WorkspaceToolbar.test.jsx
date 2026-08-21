// 工具条按钮的存废与可用性都由当前工作区决定,这里守的是这套开关规则。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render, screen } from "@testing-library/react";

import { WorkspaceToolbar } from "./WorkspaceToolbar.jsx";

const noop = () => {};

function baseProps(overrides = {}) {
  return {
    paneAvailable: true,
    collapsed: false,
    onToggleCollapsed: noop,
    navCollapsed: false,
    onToggleNav: noop,
    ...overrides,
  };
}

test("侧栏开关的提示随折叠状态切换", () => {
  const expanded = render(<WorkspaceToolbar {...baseProps()} />);
  assert.ok(screen.getByTitle("app:titlebar.collapse"));
  expanded.unmount();

  render(<WorkspaceToolbar {...baseProps({ collapsed: true })} />);
  assert.ok(screen.getByTitle("app:titlebar.expand"));
});

test("没有资源栏的工作区把侧栏开关置灰而不是移除", () => {
  const clicks = [];
  render(
    <WorkspaceToolbar
      {...baseProps({
        paneAvailable: false,
        onToggleCollapsed: () => clicks.push("toggle"),
      })}
    />,
  );

  const toggle = screen.getByTitle("app:titlebar.collapse");
  assert.equal(toggle.disabled, true);
  fireEvent.click(toggle);
  assert.deepEqual(clicks, []);
});

test("导航栏开关常驻,提示随折叠状态切换,且不受资源栏有无影响", () => {
  const clicks = [];
  const expanded = render(
    <WorkspaceToolbar
      {...baseProps({ paneAvailable: false, onToggleNav: () => clicks.push("nav") })}
    />,
  );
  const toggle = screen.getByTitle("app:nav.collapse");
  assert.equal(toggle.disabled, false);
  fireEvent.click(toggle);
  assert.deepEqual(clicks, ["nav"]);
  expanded.unmount();

  render(<WorkspaceToolbar {...baseProps({ navCollapsed: true })} />);
  assert.ok(screen.getByTitle("app:nav.expand"));
});
