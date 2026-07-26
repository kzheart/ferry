// 工作区路由:view 决定详情区渲染谁。这里守两件事——同一时刻只有一个工作区被
// 挂载,以及资料库在"没有选中会话"时走的是空态而不是崩在 detail 上。
import { test } from "vitest";
import assert from "node:assert/strict";
import { render as rtlRender, screen } from "@testing-library/react";

import { AppChromeProvider } from "../shared/capabilities/appChrome.jsx";
import { BrowserStateProvider } from "../shared/capabilities/browserState.jsx";
import { FerryRuntimeProvider } from "../shared/capabilities/ferryRuntime.jsx";
import { OperationsStateProvider } from "../shared/capabilities/operationsState.jsx";
import { SessionEditingProvider } from "../shared/capabilities/sessionEditing.jsx";
import { WorkspaceRouter } from "./WorkspaceRouter.jsx";

const noop = () => {};

// 跨工作区共享的状态已从 props 下沉为三个领域 Context。用例继续用原来的扁平
// 入参描述场景,这里按域翻译:该进 Context 的进 Context,其余仍作 props。
function render(props, { ferry, editingSurface } = {}) {
  const {
    view, sessions, scanning, navigationTarget, currentSession,
    selectedSessionId, detailMeta, detail, detailActions, onNavigate,
    onOpenConfig, environment, scan, ...rest
  } = props;
  const browserState = {
    peek: {
      current: currentSession,
      selectedId: selectedSessionId,
      meta: detailMeta,
      detail,
      actions: detailActions,
      navigationTarget,
      refreshing: detailActions?.refreshing,
      loadingMore: detailActions?.loadingMore,
    },
    search: { view, scanSessions: sessions },
    deletion: { onDeleteHistory: detailActions?.onDeleteHistory },
  };
  const operationsState = { floatChat: { onNavigate, onOpenConfig } };
  const appChrome = {
    settings: { scanning, env: environment, scanResult: scan },
  };
  let node = <WorkspaceRouter {...rest} />;
  if (editingSurface) {
    node = (
      <SessionEditingProvider value={editingSurface}>{node}</SessionEditingProvider>
    );
  }
  if (ferry) {
    node = <FerryRuntimeProvider value={ferry}>{node}</FerryRuntimeProvider>;
  }
  return rtlRender(
    <BrowserStateProvider value={browserState}>
      <OperationsStateProvider value={operationsState}>
        <AppChromeProvider value={appChrome}>{node}</AppChromeProvider>
      </OperationsStateProvider>
    </BrowserStateProvider>,
  );
}

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
  const idle = render(baseProps({ view: "library" }));
  assert.ok(screen.getByText("没有会话"));
  idle.unmount();

  render(baseProps({ view: "library", scanning: true }));
  assert.ok(screen.getByText("扫描中"));
  assert.equal(screen.queryByText("没有会话"), null);
});

test("未知 view 不渲染任何工作区", () => {
  const { container } = render(baseProps({ view: "nope" }));
  assert.equal(container.innerHTML, "");
});

test("迁移历史工作区的删除按钮接的是详情动作里的 onDeleteHistory", () => {
  const calls = [];
  render(
    baseProps({
      view: "history",
      historySelection: {
        _id: 1, id: "h1", src: "claude", dst: "codex",
        title: "一次迁移", created_at: "2026-01-01T00:00:00Z",
      },
      detailActions: {
        refreshing: false, loadingMore: false,
        onDeleteHistory: () => calls.push("delete"),
      },
    }),
  );

  const trigger = screen.getByTitle("migration:history.delete");
  trigger.click();
  assert.deepEqual(calls, ["delete"]);
});

test("对话工作区从 Context 取 Ferry Runtime 句柄,不再由路由层转交", () => {
  const ferry = {
    available: true, activeId: null, activeLog: null, sessions: [], roles: [],
    models: [], mode: "auto", health: null, lastError: null,
    selectedRoleId: "default", clearError: () => {},
  };
  render(baseProps({ view: "askferry" }), { ferry });

  assert.ok(screen.getByText("askferry:empty.title"));
});

test("资料库详情区的待应用编辑取自 Context,不再由路由层转交", () => {
  const surface = {
    scope: null, setScope: noop, ops: [], dirtyOps: [], addOp: noop, removeOp: noop,
    updateOp: noop, startReplyEdit: noop, replyEditError: () => null,
    onOpenDiff: noop, onApply: noop, applying: false, onDiscardAll: noop,
  };
  render(
    baseProps({
      view: "library",
      currentSession: { id: "s1", tool: "claude", title: "会话" },
      selectedSessionId: "s1",
      detailMeta: { id: "s1", tool: "claude", title: "会话" },
      detail: { data: { messages: [], turns: [] } },
    }),
    { editingSurface: surface },
  );

  assert.equal(screen.queryByText("没有会话"), null);
  assert.ok(screen.getByText("会话"));
});
