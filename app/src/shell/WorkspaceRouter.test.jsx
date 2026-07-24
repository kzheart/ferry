// 工作区路由:view 决定详情区渲染谁。这里守两件事——同一时刻只有一个工作区被
// 挂载,以及资料库在"没有选中会话"时走的是空态而不是崩在 detail 上。
import { test } from "vitest";
import assert from "node:assert/strict";
import { render, screen } from "@testing-library/react";

import { FerryRuntimeProvider } from "../shared/capabilities/ferryRuntime.jsx";
import { WorkspaceRouter } from "./WorkspaceRouter.jsx";

const noop = () => {};

function baseProps(overrides = {}) {
  return {
    view: "overview",
    sessions: [],
    historyRows: [],
    pricing: null,
    scanning: false,
    navigationTarget: null,
    currentSession: null,
    selectedSessionId: null,
    detailMeta: {},
    detail: null,
    detailActions: { refreshing: false, loadingMore: false, onDeleteHistory: noop },
    scope: "all",
    ops: [],
    dirtyOps: [],
    applying: false,
    historySelection: null,
    ferry: null,
    agentAttachments: [],
    onAgentAttachmentsChange: noop,
    onNavigate: noop,
    onOpenConfig: noop,
    environment: {},
    scan: null,
    onFirstDone: noop,
    scanningLabel: "扫描中",
    emptyLibraryLabel: "没有会话",
    ...overrides,
  };
}

test("资料库没有选中会话时按是否在扫描给出不同空态", () => {
  const idle = render(<WorkspaceRouter {...baseProps({ view: "library" })} />);
  assert.ok(screen.getByText("没有会话"));
  idle.unmount();

  render(<WorkspaceRouter {...baseProps({ view: "library", scanning: true })} />);
  assert.ok(screen.getByText("扫描中"));
  assert.equal(screen.queryByText("没有会话"), null);
});

test("未知 view 不渲染任何工作区", () => {
  const { container } = render(<WorkspaceRouter {...baseProps({ view: "nope" })} />);
  assert.equal(container.innerHTML, "");
});

test("迁移历史工作区的删除按钮接的是详情动作里的 onDeleteHistory", () => {
  const calls = [];
  render(
    <WorkspaceRouter
      {...baseProps({
        view: "history",
        historySelection: {
          _id: 1, id: "h1", src: "claude", dst: "codex",
          title: "一次迁移", created_at: "2026-01-01T00:00:00Z",
        },
        detailActions: {
          refreshing: false, loadingMore: false,
          onDeleteHistory: () => calls.push("delete"),
        },
      })}
    />,
  );

  const trigger = screen.getByTitle("migration:history.delete");
  trigger.click();
  assert.deepEqual(calls, ["delete"]);
});

test("首次运行工作区把环境与扫描结果透传给 FirstRun", () => {
  const { container } = render(
    <WorkspaceRouter
      {...baseProps({ view: "firstrun", environment: { claude: true }, scan: { sessions: [] } })}
    />,
  );
  assert.notEqual(container.innerHTML, "");
});

test("对话工作区从 Context 取 Ferry Runtime 句柄,不再由路由层转交", () => {
  const ferry = {
    available: true, activeId: null, activeLog: null, sessions: [], roles: [],
    models: [], mode: "auto", health: null, lastError: null,
    selectedRoleId: "default", clearError: () => {},
  };
  render(
    <FerryRuntimeProvider value={ferry}>
      <WorkspaceRouter {...baseProps({ view: "askferry" })} />
    </FerryRuntimeProvider>,
  );

  assert.ok(screen.getByText("askferry:empty.title"));
});
