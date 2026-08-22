// 工具条只剩一颗侧栏开关:导航栏与资源栏一起收,提示随收起状态切换。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render, screen } from "@testing-library/react";

import { WorkspaceToolbar } from "./WorkspaceToolbar.jsx";

test("侧栏开关的提示随收起状态切换", () => {
  const expanded = render(<WorkspaceToolbar collapsed={false} onToggle={() => {}} />);
  assert.ok(screen.getByTitle("app:titlebar.collapse"));
  expanded.unmount();

  render(<WorkspaceToolbar collapsed onToggle={() => {}} />);
  assert.ok(screen.getByTitle("app:titlebar.expand"));
});

test("点开关把状态交回给上层,自己不留状态", () => {
  const clicks = [];
  render(<WorkspaceToolbar collapsed={false} onToggle={() => clicks.push("toggle")} />);
  const toggle = screen.getByTitle("app:titlebar.collapse");
  assert.equal(toggle.getAttribute("aria-pressed"), "true");
  fireEvent.click(toggle);
  assert.deepEqual(clicks, ["toggle"]);
});
