// 资源栏宿主:同一个外壳(标题/搜索/筛选/令牌)套三种列表。这里守的是外壳与列表
// 的接线,以及"折叠只是宽度归零、内容仍在"这一点——折叠时如果卸载了列表,
// 展开会丢滚动位置和展开态。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render as rtlRender, screen } from "@testing-library/react";

import { FerryRuntimeProvider } from "../shared/capabilities/ferryRuntime.jsx";
import { ResourcePaneHost } from "./ResourcePaneHost.jsx";

// 对话列表的打开/新建/置顶/删除直接取 Ferry Runtime 句柄。
const ferry = {
  activeId: null, openSession: () => {}, newChat: () => {},
  pin: async () => {}, deleteSession: async () => {}, reportError: () => {},
};

const render = ui => rtlRender(<FerryRuntimeProvider value={ferry}>{ui}</FerryRuntimeProvider>);

const noop = () => {};

function baseProps(overrides = {}) {
  return {
    view: "library",
    pane: {
      title: "资料库", count: 2, placeholder: "搜索会话",
      query: "", onQuery: noop, filterCount: 0, footer: null, tokens: [],
    },
    collapsed: false,
    width: 260,
    resizing: false,
    filterOpen: false,
    onOpenSearch: noop,
    onFilter: noop,
    library: {
      scanning: false, sessions: [], scanningLabel: "扫描中",
      groups: [], collapsedGroups: {}, onToggleGroup: noop, onClear: noop,
      selectedId: null, multiSel: [],
      onRowClick: noop, onRowPin: noop, onRowDelete: noop, onRowMore: noop,
    },
    history: { groups: [], filtered: [], onDelete: noop, onClear: noop },
    agent: { sessions: [], onRename: noop },
    ...overrides,
  };
}

test("外壳渲染标题与计数,并把搜索、筛选按钮接到各自的回调", () => {
  const calls = [];
  render(
    <ResourcePaneHost
      {...baseProps({
        onOpenSearch: () => calls.push("search"),
        onFilter: () => calls.push("filter"),
      })}
    />,
  );

  assert.ok(screen.getByText("资料库"));
  assert.ok(screen.getByText("2"));
  fireEvent.click(screen.getByTitle("app:pane.search"));
  fireEvent.click(screen.getByTitle("app:pane.filterButton"));
  assert.deepEqual(calls, ["search", "filter"]);
});

test("已有查询词时展示可清除的查询条,清除走的是 pane.onQuery", () => {
  const queries = [];
  render(
    <ResourcePaneHost
      {...baseProps({
        pane: { ...baseProps().pane, query: "重构", onQuery: e => queries.push(e.target.value) },
      })}
    />,
  );

  assert.ok(screen.getByText("重构"));
  fireEvent.click(screen.getByTitle("common:empty.clearFilter"));
  assert.deepEqual(queries, [""]);
});

test("筛选令牌逐个可移除", () => {
  const removed = [];
  render(
    <ResourcePaneHost
      {...baseProps({
        pane: {
          ...baseProps().pane,
          tokens: [
            { label: "仅 Claude", onRemove: () => removed.push("tool") },
            { label: "近 7 天", onRemove: () => removed.push("time") },
          ],
        },
      })}
    />,
  );

  fireEvent.click(screen.getByText("近 7 天").querySelector("a"));
  assert.deepEqual(removed, ["time"]);
});

test("资料库首扫且尚无会话时占位,扫完有会话就换成列表", () => {
  const scanning = render(
    <ResourcePaneHost
      {...baseProps({
        library: { ...baseProps().library, scanning: true, sessions: [] },
      })}
    />,
  );
  assert.ok(screen.getByText("扫描中"));
  scanning.unmount();

  // 后台重扫时已有会话,不能把列表换成占位——那会让用户眼前的列表突然消失。
  render(
    <ResourcePaneHost
      {...baseProps({
        library: {
          ...baseProps().library,
          scanning: true,
          sessions: [{ id: "a" }],
          groups: [{ key: "g", label: "ferry", rows: [] }],
        },
      })}
    />,
  );
  assert.equal(screen.queryByText("扫描中"), null);
});

test("view 决定挂载哪一种列表", () => {
  const agent = {
    ...baseProps().agent,
    sessions: [{ session_id: "f1", title: "一次问询", model_id: "opus" }],
  };
  const library = render(<ResourcePaneHost {...baseProps({ view: "library" })} />);
  assert.equal(screen.queryByText("一次问询"), null);
  library.unmount();

  render(<ResourcePaneHost {...baseProps({ view: "askferry", agent })} />);
  assert.ok(screen.getByText("一次问询"));
});

test("折叠只把宽度收成 0,列表内容仍然挂载", () => {
  const agent = {
    ...baseProps().agent,
    sessions: [{ session_id: "f1", title: "一次问询", model_id: "opus" }],
  };
  const { container } = render(
    <ResourcePaneHost {...baseProps({ view: "askferry", agent, collapsed: true })} />,
  );

  assert.equal(container.firstChild.style.width, "0px");
  assert.ok(screen.getByText("一次问询"));
});
