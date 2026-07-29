// AppOverlayController 把主壳的状态翻译成弹层入参,这里有真正的推导:搜索结果的
// 三种视图形态、标签的初始值与提交语义、删除历史的先清选中再删。
// 下面的用例锁的是这些推导,不是渲染骨架(骨架在 AppOverlays.test.jsx)。
import { test, vi } from "vitest";
import assert from "node:assert/strict";
import { act, fireEvent, render as rtlRender, screen } from "@testing-library/react";

// ⌘K 的全文匹配段要真的打 Engine,这里把 engine() 换成可控桩
let engineResponse = null;
const engineCalls = [];
vi.mock("../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  engine: async (method, params) => {
    engineCalls.push([method, params]);
    return engineResponse;
  },
}));

import { AppChromeProvider } from "../shared/capabilities/appChrome.jsx";
import { BrowserStateProvider } from "../shared/capabilities/browserState.jsx";
import { OperationsStateProvider } from "../shared/capabilities/operationsState.jsx";
import { FerryRuntimeProvider } from "../shared/capabilities/ferryRuntime.jsx";
import { SessionEditingProvider } from "../shared/capabilities/sessionEditing.jsx";
import { AppOverlayController } from "./AppOverlayController.jsx";

const noop = () => {};

// 控制器直接读 Ferry Runtime 句柄(打开会话、重命名会话),所以渲染必须带上
// Provider。每个用例按需替换句柄上的某个方法来观察调用。
let ferry = null;

const editingSurface = {
  scope: null, setScope: noop, ops: [], dirtyOps: [], addOp: noop, removeOp: noop,
  updateOp: noop, startReplyEdit: noop, replyEditError: () => null,
  onOpenDiff: noop, onApply: noop, applying: false, onDiscardAll: noop,
};

// browser 域的入参已从 props 下沉为 Context。用例仍按命名空间构造同一份数据,
// 这里按 key 分流:browser 那几个进 Provider,其余照旧作为 props。
const BROWSER_STATE_KEYS = [
  "peek", "search", "contextMenu", "deletion", "tags", "filters",
];
const OPERATIONS_STATE_KEYS = [
  "migration", "editing", "floatChat",
];
const APP_CHROME_KEYS = ["toast", "railTip", "settings", "guide"];

const pick = (props, keys) =>
  Object.fromEntries(keys.filter(key => key in props).map(key => [key, props[key]]));

function render(ui) {
  const wrap = node => (
    <FerryRuntimeProvider value={ferry}>
      <SessionEditingProvider value={editingSurface}>
        <BrowserStateProvider value={pick(node.props, BROWSER_STATE_KEYS)}>
          <OperationsStateProvider
            value={pick(node.props, OPERATIONS_STATE_KEYS)}
          >
            <AppChromeProvider value={pick(node.props, APP_CHROME_KEYS)}>
              {node}
            </AppChromeProvider>
          </OperationsStateProvider>
        </BrowserStateProvider>
      </SessionEditingProvider>
    </FerryRuntimeProvider>
  );
  const result = rtlRender(wrap(ui));
  return { ...result, rerender: node => result.rerender(wrap(node)) };
}

function stubFerry(overrides = {}) {
  ferry = { openSession: noop, rename: async () => {}, reportError: noop, ...overrides };
  return ferry;
}

stubFerry();

// 与 workspaceOverlayProps.buildOverlayProps 的输出同形。
function baseProps(overrides = {}) {
  return {
    t: key => key,
    floatChat: { mounted: false },
    peek: { id: null, current: null, setId: noop, setView: noop },
    migration: { state: null, current: null, settings: {}, setState: noop, loadHistory: noop },
    editing: { diff: null, dirtyOps: [], confirmApply: false, setDiff: noop, setConfirmApply: noop, apply: noop },
    search: {
      open: false, pane: null, view: "library",
      ferrySessions: [], historyGroups: [], libraryGroups: [],
      selectHistory: noop, setMultiSelection: noop, selectSession: noop, setOpen: noop,
    },
    contextMenu: { value: null, items: null, setValue: noop },
    deletion: {
      sessionConfirmation: null, batchConfirmation: null,
      cancelSessionDeletion: noop, confirmSessionDeletion: noop,
      cancelBatchDeletion: noop, confirmBatchDeletion: noop,
      history: null, setHistory: noop, deleteHistory: async () => {},
      selectedHistoryId: null, selectHistory: noop,
    },
    tags: { selection: null, setSelection: noop, metaFor: () => ({}), updateMetadata: noop },
    toast: { value: null, setValue: noop },
    railTip: { value: null, railOnly: false },
    settings: { open: false, value: {}, setOpen: noop, setView: noop, openGuide: noop },
    filters: {
      popover: null, anchor: null, onClose: noop,
      library: { value: null, onChange: noop, counts: {}, dirs: [], tags: [], onClear: noop },
      history: {
        value: null,
        tools: ["claude", "codex", "opencode", "pi", "grok"],
        onChange: noop,
        onClear: noop,
      },
    },
    guide: { step: 0, onGo: noop, onFinish: noop },
    ...overrides,
  };
}

const searchPane = { placeholder: "搜索", query: "", onQuery: noop };

test("资料库视图的搜索结果先清空多选再选中会话", () => {
  const calls = [];
  render(
    <AppOverlayController
      {...baseProps({
        search: {
          ...baseProps().search,
          open: true,
          pane: searchPane,
          view: "library",
          libraryGroups: [
            { rows: [{ key: "claude:a", title: "会话 A", tool: "claude", repo: "ferry" }] },
          ],
          setMultiSelection: value => calls.push(["multi", value]),
          selectSession: key => calls.push(["select", key]),
        },
      })}
    />,
  );

  fireEvent.click(screen.getByText("会话 A"));
  assert.deepEqual(calls, [["multi", []], ["select", "claude:a"]]);
});

test("历史视图的搜索结果展平分组并拼出迁移方向", () => {
  const selected = [];
  render(
    <AppOverlayController
      {...baseProps({
        search: {
          ...baseProps().search,
          open: true,
          pane: searchPane,
          view: "history",
          historyGroups: [
            { rows: [{ id: "h1", title: "迁移一", tool: "claude", from: "claude", to: "codex" }] },
            { rows: [{ id: "h2", title: "迁移二", tool: "codex", from: "codex", to: "opencode" }] },
          ],
          selectHistory: id => selected.push(id),
        },
      })}
    />,
  );

  assert.ok(screen.getByText("claude → codex"));
  fireEvent.click(screen.getByText("迁移二"));
  assert.deepEqual(selected, ["h2"]);
});

test("Ask Ferry 视图的无标题会话回落到占位文案", () => {
  const opened = [];
  stubFerry({ openSession: id => opened.push(id) });
  render(
    <AppOverlayController
      {...baseProps({
        search: {
          ...baseProps().search,
          open: true,
          pane: searchPane,
          view: "askferry",
          ferrySessions: [{ session_id: "f1", title: "", model_id: "opus" }],
        },
      })}
    />,
  );

  fireEvent.click(screen.getByText("askferry:chat.untitled"));
  assert.deepEqual(opened, ["f1"]);
});

test("搜索结果截断到 60 条", () => {
  render(
    <AppOverlayController
      {...baseProps({
        search: {
          ...baseProps().search,
          open: true,
          pane: searchPane,
          view: "library",
          libraryGroups: [
            {
              rows: Array.from({ length: 80 }, (_, index) => ({
                key: `k${index}`, title: `会话 ${index}`, tool: "claude", repo: "ferry",
              })),
            },
          ],
        },
      })}
    />,
  );

  assert.ok(screen.getByText("会话 59"));
  assert.equal(screen.queryByText("会话 60"), null);
});

// sessionIdentity 用 NUL 拼 tool 与 id,源码里不直接写这个字节
const idKey = id => "claude" + String.fromCharCode(0) + id;

// 全文检索有 300ms 防抖,等它过去并让 state 落定
const settleSearch = () =>
  act(async () => { await new Promise(resolve => setTimeout(resolve, 360)); });

const contentSearchProps = (overrides = {}) => ({
  ...baseProps().search,
  open: true,
  pane: { ...searchPane, query: "检索" },
  view: "library",
  scanSessions: [
    { tool: "claude", id: "a", ref: "fsr_a" },
    { tool: "claude", id: "b", ref: "fsr_b" },
  ],
  libraryGroups: [
    { rows: [{ key: idKey("a"), title: "会话 A", tool: "claude", repo: "ferry" }] },
  ],
  ...overrides,
});

test("资料库视图追加全文匹配段,已在标题结果里的会话不重复出现", async () => {
  engineCalls.length = 0;
  engineResponse = {
    query: "检索",
    scope: "content",
    sessions: [
      { tool: "claude", ref: "fsr_a", title: "会话 A", snippet: "A 的正文" },
      { tool: "claude", ref: "fsr_b", title: "会话 B", snippet: "命中的正文片段",
        project: "ferry" },
    ],
    content_index: { ready: true },
  };
  const calls = [];
  render(
    <AppOverlayController
      {...baseProps({
        search: contentSearchProps({
          setMultiSelection: value => calls.push(["multi", value]),
          selectSession: key => calls.push(["select", key]),
        }),
      })}
    />,
  );
  await settleSearch();

  assert.deepEqual(engineCalls, [
    ["session_search", { query: "检索", scope: "content", limit: 20 }],
  ]);
  // 前端过滤已经给出 A,全文段只补 B
  assert.equal(screen.getAllByText("会话 A").length, 1);
  assert.ok(screen.getByText("app:search.fullText"));
  assert.ok(screen.getByText("命中的正文片段"));
  assert.ok(screen.getByText("app:search.contentHit"));

  // 点全文结果与点前端结果同样是先清多选再按 identity key 选中
  fireEvent.click(screen.getByText("会话 B"));
  assert.deepEqual(calls, [["multi", []], ["select", idKey("b")]]);
});

test("过期响应被丢弃:回显 query 与当前输入对不上就不渲染", async () => {
  engineCalls.length = 0;
  engineResponse = {
    query: "上一次的输入",
    sessions: [
      { tool: "claude", ref: "fsr_b", title: "会话 B", snippet: "不该出现" },
    ],
    content_index: { ready: true },
  };
  render(<AppOverlayController {...baseProps({ search: contentSearchProps() })} />);
  await settleSearch();

  assert.equal(screen.queryByText("app:search.fullText"), null);
  assert.equal(screen.queryByText("不该出现"), null);
});

test("内容索引未就绪时给出构建中提示,而不是当成无结果", async () => {
  engineCalls.length = 0;
  engineResponse = {
    query: "检索",
    sessions: [],
    content_index: { ready: false },
  };
  render(
    <AppOverlayController
      {...baseProps({ search: contentSearchProps({ libraryGroups: [] }) })}
    />,
  );
  await settleSearch();

  assert.ok(screen.getByText("app:search.indexing"));
  assert.equal(screen.queryByText("app:search.empty"), null);
});

test("单会话标签弹层带出已有标签,提交时整组替换", async () => {
  const written = [];
  render(
    <AppOverlayController
      {...baseProps({
        tags: {
          selection: { batch: false, sessions: [{ id: "s1" }] },
          setSelection: noop,
          metaFor: () => ({ tags: ["旧一", "旧二"] }),
          updateMetadata: (session, patch) => { written.push([session.id, patch.tags]); },
        },
      })}
    />,
  );

  const input = screen.getByRole("textbox");
  assert.equal(input.value, "旧一, 旧二");
  fireEvent.change(input, { target: { value: "新一，新二 ,, " } });
  fireEvent.click(screen.getByText("app:prompt.save"));
  await Promise.resolve();

  // 中英文逗号都算分隔符,空项丢弃;非批量是替换语义,旧标签不保留。
  assert.deepEqual(written, [["s1", ["新一", "新二"]]]);
});

test("批量标签弹层不带初始值,提交时逐个会话并入且去重", async () => {
  const written = [];
  const existing = { s1: ["共有"], s2: ["共有", "独有"] };
  render(
    <AppOverlayController
      {...baseProps({
        tags: {
          selection: { batch: true, sessions: [{ id: "s1" }, { id: "s2" }] },
          setSelection: noop,
          metaFor: session => ({ tags: existing[session.id] }),
          updateMetadata: async (session, patch) => { written.push([session.id, patch.tags]); },
        },
      })}
    />,
  );

  assert.equal(screen.getByRole("textbox").value, "");
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "共有, 新增" } });
  fireEvent.click(screen.getByText("app:prompt.save"));
  await new Promise(resolve => setTimeout(resolve, 0));

  assert.deepEqual(written, [
    ["s1", ["共有", "新增"]],
    ["s2", ["共有", "独有", "新增"]],
  ]);
});

test("删除正在查看的迁移历史时先清空选中", () => {
  const calls = [];
  render(
    <AppOverlayController
      {...baseProps({
        deletion: {
          ...baseProps().deletion,
          history: { _id: 7, id: "h7", src: "claude", dst: "codex" },
          selectedHistoryId: 7,
          selectHistory: value => calls.push(["selectHistory", value]),
          setHistory: value => calls.push(["close", value]),
          deleteHistory: async id => { calls.push(["delete", id]); },
        },
      })}
    />,
  );

  fireEvent.click(screen.getByText("overlays:historyDelete.confirm"));
  assert.deepEqual(calls, [
    ["selectHistory", null],
    ["close", null],
    ["delete", "h7"],
  ]);
});

test("删除未选中的迁移历史不动选中状态", () => {
  const calls = [];
  render(
    <AppOverlayController
      {...baseProps({
        deletion: {
          ...baseProps().deletion,
          history: { _id: 7, id: "h7", src: "claude", dst: "codex" },
          selectedHistoryId: 9,
          selectHistory: value => calls.push(["selectHistory", value]),
          setHistory: value => calls.push(["close", value]),
          deleteHistory: async id => { calls.push(["delete", id]); },
        },
      })}
    />,
  );

  fireEvent.click(screen.getByText("overlays:historyDelete.confirm"));
  assert.deepEqual(calls, [["close", null], ["delete", "h7"]]);
});

test("就地预览要同时有 id 和已加载的会话才打开", () => {
  const { container, rerender } = render(
    <AppOverlayController
      {...baseProps({ peek: { ...baseProps().peek, id: "s1", current: null } })}
    />,
  );
  assert.equal(container.innerHTML, "");

  rerender(
    <AppOverlayController
      {...baseProps({
        peek: {
          ...baseProps().peek,
          id: "s1",
          current: { id: "s1", tool: "claude", title: "会话" },
          selectedId: "s1",
          meta: { id: "s1", tool: "claude", title: "会话" },
          detail: null,
          actions: {},
        },
      })}
    />,
  );
  assert.notEqual(container.innerHTML, "");
});

test("筛选弹层由 popover 的取值区分资料库与历史", () => {
  const props = baseProps();
  const filters = {
    ...props.filters,
    library: { ...props.filters.library, value: { src: [], time: "all", dir: null, mig: false, sub: false, tag: null } },
    history: { ...props.filters.history, value: { src: [], time: "all" } },
  };
  const { rerender } = render(
    <AppOverlayController {...baseProps({ filters: { ...filters, popover: "lib" } })} />,
  );
  assert.ok(screen.getByText("overlays:filter.projectDir"));

  rerender(<AppOverlayController {...baseProps({ filters: { ...filters, popover: "hist" } })} />);
  assert.equal(screen.queryByText("overlays:filter.projectDir"), null);
});
