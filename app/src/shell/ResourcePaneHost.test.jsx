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
      query: "", onQuery: noop, filterCount: 0, tokens: [],
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
    agent: { sessions: [] },
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

// 空列表有两种成因,说错了就是把"你还没接入任何工具"讲成"你筛过头了"。
test("没有筛选的空资料库给出路,而不是一个点了没反应的清除筛选", () => {
  const rescans = [];
  render(
    <ResourcePaneHost
      {...baseProps({
        library: { ...baseProps().library, groups: [], onRescan: () => rescans.push(1) },
      })}
    />,
  );

  assert.ok(screen.getByText("common:empty.libraryNone"));
  assert.ok(screen.getByText("common:empty.libraryNoneHint"));
  assert.equal(screen.queryByText("common:empty.library"), null, "不该说'没有匹配'");
  // 清除筛选按钮不存在(顶部查询条也没有,所以整屏都不该出现这个词)
  assert.equal(screen.queryByText("common:empty.clearFilter"), null);

  fireEvent.click(screen.getByText("common:empty.rescan"));
  assert.deepEqual(rescans, [1]);
});

test("有查询词时的空资料库才给清除筛选", () => {
  const cleared = [];
  render(
    <ResourcePaneHost
      {...baseProps({
        pane: { ...baseProps().pane, query: "重构" },
        library: { ...baseProps().library, groups: [], onClear: () => cleared.push(1) },
      })}
    />,
  );

  assert.ok(screen.getByText("common:empty.library"));
  assert.equal(screen.queryByText("common:empty.libraryNone"), null);
  // 顶部查询条那个清除按钮只有 title 没有文本,getByText 命中的就是空态里的
  fireEvent.click(screen.getByText("common:empty.clearFilter"));
  assert.deepEqual(cleared, [1]);
});

// 侧栏只按标题匹配,全文检索在 ⌘K 面板里——搜不到的人不该靠猜才知道还有另一条路。
test("侧栏搜不到时给出全文搜索的直达入口", () => {
  const opened = [];
  render(
    <ResourcePaneHost
      {...baseProps({
        pane: { ...baseProps().pane, query: "重构" },
        onOpenSearch: () => opened.push(1),
        library: { ...baseProps().library, groups: [] },
      })}
    />,
  );

  assert.ok(screen.getByText("common:empty.titleOnlyHint"));
  fireEvent.click(screen.getByText("common:empty.fullTextSearch"));
  assert.deepEqual(opened, [1]);
});

test("只有筛选条件、没有查询词时不提全文搜索——没有词可搜", () => {
  render(
    <ResourcePaneHost
      {...baseProps({
        pane: { ...baseProps().pane, filterCount: 2 },
        library: { ...baseProps().library, groups: [] },
      })}
    />,
  );

  assert.equal(screen.queryByText("common:empty.fullTextSearch"), null);
  assert.equal(screen.queryByText("common:empty.titleOnlyHint"), null);
  assert.ok(screen.getByText("common:empty.clearFilter"));
});

// 扫描失败时列表也是空的,但那是故障,不是"没匹配上"或"你还没有会话"。
test("扫描失败的空列表说清是故障,带错误详情和重试", () => {
  const rescans = [];
  render(
    <ResourcePaneHost
      {...baseProps({
        library: {
          ...baseProps().library,
          groups: [],
          scanError: "permission denied: ~/.claude/projects",
          onRescan: () => rescans.push(1),
        },
      })}
    />,
  );

  assert.ok(screen.getByText("common:empty.scanFailed"));
  assert.ok(screen.getByText("permission denied: ~/.claude/projects"));
  assert.equal(screen.queryByText("common:empty.libraryNone"), null, "不该说成'还没有会话'");

  fireEvent.click(screen.getByText("common:empty.retryScan"));
  assert.deepEqual(rescans, [1]);
});

test("扫描失败压过筛选:有查询词也先说故障,而不是'没有匹配'", () => {
  render(
    <ResourcePaneHost
      {...baseProps({
        pane: { ...baseProps().pane, query: "重构" },
        library: { ...baseProps().library, groups: [], scanError: "engine offline" },
      })}
    />,
  );

  assert.ok(screen.getByText("common:empty.scanFailed"));
  assert.equal(screen.queryByText("common:empty.library"), null);
});

test("扫描失败但上次结果还在:列表照常渲染,顶部说明这是旧数据", () => {
  const rescans = [];
  render(
    <ResourcePaneHost
      {...baseProps({
        library: {
          ...baseProps().library,
          sessions: [{ id: "a" }],
          groups: [{ key: "g", label: "今天", count: 1, rows: [
            { key: "claude:a", tool: "claude", title: "旧会话", dir: "/tmp", active: "刚刚" },
          ] }],
          scanError: "engine offline",
          onRescan: () => rescans.push(1),
        },
      })}
    />,
  );

  assert.ok(screen.getByText("旧会话"), "列表仍然可用");
  assert.ok(screen.getByText("common:empty.staleScan"));
  assert.ok(screen.getByText("engine offline"));
  fireEvent.click(screen.getByText("common:empty.retryScan"));
  assert.deepEqual(rescans, [1]);
});

test("扫描成功时既没有旧数据提示,也没有故障空态", () => {
  render(<ResourcePaneHost {...baseProps({ library: { ...baseProps().library, groups: [] } })} />);

  assert.equal(screen.queryByText("common:empty.staleScan"), null);
  assert.equal(screen.queryByText("common:empty.scanFailed"), null);
  assert.ok(screen.getByText("common:empty.libraryNone"));
});

test("只有筛选条件(无查询词)同样算筛出来的空", () => {
  render(
    <ResourcePaneHost
      {...baseProps({
        pane: { ...baseProps().pane, filterCount: 2 },
        library: { ...baseProps().library, groups: [] },
      })}
    />,
  );

  assert.ok(screen.getByText("common:empty.library"));
});

test("迁移历史的真空态讲清楚记录从哪来,且不给清除筛选", () => {
  render(
    <ResourcePaneHost
      {...baseProps({ view: "history", history: { groups: [], filtered: [], onDelete: noop, onClear: noop } })}
    />,
  );

  assert.ok(screen.getByText("common:empty.historyNone"));
  assert.ok(screen.getByText("common:empty.historyNoneHint"));
  assert.equal(screen.queryByText("common:empty.clearFilter"), null);
});
