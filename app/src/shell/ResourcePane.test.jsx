// 资源栏的两处交互细节:选中行的滚动跟随,以及行内重命名的失焦语义。
// 列表是虚拟化的,视口外的行不挂载——滚动跟随一旦缺失,键盘 ↑/↓ 翻过视口后
// 高亮就"消失"了(既不在屏幕上,也没有 DOM 可循)。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render, screen } from "@testing-library/react";

import { TOOL_NAME } from "../shared/contracts/tools.js";

import { LibraryList, Pane, nextFocusScrollTop } from "./ResourcePane.jsx";

// ---- 滚动跟随的位置计算(jsdom 不做布局,只能在纯函数上断言) ----

const viewport = { scrollTop: 100, viewHeight: 200 }; // 视口覆盖 100..300

test("目标行已在视口内时不滚动,不打断用户自己的滚动位置", () => {
  const next = nextFocusScrollTop({ itemTop: 150, itemHeight: 30, ...viewport });
  assert.equal(next, null);
});

test("目标行在视口上方时贴上沿,留出一点余白", () => {
  const next = nextFocusScrollTop({ itemTop: 40, itemHeight: 30, ...viewport });
  assert.equal(next, 32); // 40 - 8
});

test("目标行在视口下方时贴下沿,行底完整可见", () => {
  const next = nextFocusScrollTop({ itemTop: 400, itemHeight: 30, ...viewport });
  assert.equal(next, 238); // 400 + 30 - 200 + 8
});

test("列表顶部的行不会算出负的 scrollTop", () => {
  const next = nextFocusScrollTop({
    itemTop: 0, itemHeight: 30, scrollTop: 50, viewHeight: 200,
  });
  assert.equal(next, 0);
});

// ---- 行内重命名:失焦即提交,而不是丢弃已输入的内容 ----

const row = { key: "claude:a", tool: "claude", title: "原标题", dir: "/tmp", active: "刚刚" };

function renderRenaming(handlers) {
  const groups = [{ key: "today", label: "今天", count: 1, rows: [row] }];
  return render(
    <LibraryList
      groups={groups}
      collapsed={{}}
      onToggle={() => {}}
      empty={false}
      onClear={() => {}}
      selectedId={row.key}
      multiSel={[]}
      renamingKey={row.key}
      onRowClick={() => {}}
      onRowPin={() => {}}
      onRowDelete={() => {}}
      onRowMore={() => {}}
      onRowRename={() => {}}
      onRowRenameSubmit={() => {}}
      onRowRenameCancel={() => {}}
      {...handlers}
    />,
  );
}

test("重命名输入框失焦时提交已输入的内容,而不是丢弃", () => {
  const submitted = [];
  let cancelled = false;
  renderRenaming({
    onRowRenameSubmit: (key, value) => submitted.push([key, value]),
    onRowRenameCancel: () => { cancelled = true; },
  });

  const input = screen.getByPlaceholderText("app:prompt.renamePlaceholder");
  fireEvent.change(input, { target: { value: "新标题" } });
  fireEvent.blur(input);

  assert.deepEqual(submitted, [["claude:a", "新标题"]]);
  assert.equal(cancelled, false);
});

test("Esc 取消后紧随的失焦不再提交一次", () => {
  const submitted = [];
  let cancelled = false;
  renderRenaming({
    onRowRenameSubmit: (key, value) => submitted.push([key, value]),
    onRowRenameCancel: () => { cancelled = true; },
  });

  const input = screen.getByPlaceholderText("app:prompt.renamePlaceholder");
  fireEvent.change(input, { target: { value: "半途改的名字" } });
  fireEvent.keyDown(input, { key: "Escape" });
  fireEvent.blur(input);

  assert.equal(cancelled, true);
  assert.deepEqual(submitted, []);
});

test("Enter 提交后紧随的失焦不会重复提交", () => {
  const submitted = [];
  renderRenaming({ onRowRenameSubmit: (key, value) => submitted.push([key, value]) });

  const input = screen.getByPlaceholderText("app:prompt.renamePlaceholder");
  fireEvent.change(input, { target: { value: "新标题" } });
  fireEvent.keyDown(input, { key: "Enter" });
  fireEvent.blur(input);

  assert.deepEqual(submitted, [["claude:a", "新标题"]]);
});

// ---- 双行行、项目文件夹与头部控制条 ----

const sessionRow = {
  key: "claude:a", tool: "claude", title: "支付重构", repo: "payments",
  dir: "/work/payments", active: "刚刚", count: 12,
};

function renderLibrary(props) {
  return render(
    <LibraryList
      groups={[{ key: "today", kind: "time", label: "今天", count: 1, rows: [sessionRow] }]}
      collapsed={{}}
      onToggle={() => {}}
      empty={false}
      query=""
      onClear={() => {}}
      selectedId={null}
      multiSel={[]}
      renamingKey={null}
      onRowClick={() => {}}
      onRowPin={() => {}}
      onRowDelete={() => {}}
      onRowMore={() => {}}
      onRowRename={() => {}}
      onRowRenameSubmit={() => {}}
      onRowRenameCancel={() => {}}
      {...props}
    />,
  );
}

test("会话行是双行:标题一行,项目 · Agent · 条数一行", () => {
  renderLibrary();

  assert.ok(screen.getByText("支付重构"));
  const meta = screen.getByText(/payments · /);
  assert.ok(meta.textContent.includes(TOOL_NAME.claude));
  assert.ok(meta.textContent.includes("app:library.metaCount"));
});

test("选定项目范围后元信息不再重复项目名", () => {
  renderLibrary({ scopeKind: "project" });

  assert.equal(screen.queryByText(/payments · /), null);
  assert.ok(screen.getByText(new RegExp(`^${TOOL_NAME.claude} · `)));
});

test("选定 Agent 范围后元信息不再重复 Agent 名", () => {
  renderLibrary({ scopeKind: "agent" });

  const meta = screen.getByText(/^payments · /);
  assert.equal(meta.textContent.includes(TOOL_NAME.claude), false);
});

const projectGroup = (rows = [sessionRow]) => ({
  key: "dir:/work/payments", kind: "project", label: "payments", parent: "/work",
  dir: "/work/payments", tools: ["claude", "codex"], count: rows.length, rows,
});

test("项目文件夹头只有文件夹名与计数,完整路径落在 title 上", () => {
  const toggled = [];
  renderLibrary({
    groupMode: "project",
    collapsed: { "dir:/work/payments": false },
    groups: [projectGroup()],
    onToggle: (key, kind) => toggled.push([key, kind]),
  });

  const folder = screen.getByText("payments");
  assert.equal(folder.closest("[title]").getAttribute("title"), "/work/payments");
  // 淡色父路径与 agent 圆点都已撤掉
  assert.equal(screen.queryByText("/work"), null);
  fireEvent.click(folder);
  assert.deepEqual(toggled, [["dir:/work/payments", "project"]]);
});

test("项目文件夹默认折叠,没有折叠记录时不渲染组内会话行", () => {
  renderLibrary({ groupMode: "project", groups: [projectGroup()] });

  assert.equal(screen.queryByText("支付重构"), null);
});

test("展开记录写进 collapsed 后组内会话行才出现", () => {
  renderLibrary({
    groupMode: "project",
    collapsed: { "dir:/work/payments": false },
    groups: [projectGroup()],
  });

  assert.ok(screen.getByText("支付重构"));
});

// 「不分组」是一整条按更新时间排的列表:没有分组头,也不受折叠状态影响
test("不分组的列表直接铺开会话行,不渲染分组头", () => {
  renderLibrary({
    groupMode: "none",
    collapsed: { flat: true },
    groups: [{ key: "flat", kind: "flat", label: "", count: 1, rows: [sessionRow] }],
  });

  assert.ok(screen.getByText("支付重构"));
});

// ---- 文件夹头的悬停动作与范围返回 ----

test("文件夹头带 ☆ 收藏与 → 只看此项目两个动作,已收藏时星是实心的", () => {
  const favorited = [];
  const scoped = [];
  renderLibrary({
    groupMode: "project",
    groups: [projectGroup()],
    favorites: ["/work/payments"],
    onFavoriteProject: dir => favorited.push(dir),
    onOnlyProject: dir => scoped.push(dir),
  });

  const star = screen.getByLabelText("app:ctx.unfavoriteProject");
  const only = screen.getByLabelText("app:ctx.onlyThisProject");
  // 已收藏:实心星(path 有 fill)
  assert.ok(star.querySelector("path").getAttribute("fill") === "currentColor");

  fireEvent.click(star);
  fireEvent.click(only);
  assert.deepEqual(favorited, ["/work/payments"]);
  assert.deepEqual(scoped, ["/work/payments"]);
});

test("未收藏时星是空心的,点一下就是收藏", () => {
  const favorited = [];
  renderLibrary({
    groupMode: "project",
    groups: [projectGroup()],
    favorites: [],
    onFavoriteProject: dir => favorited.push(dir),
    onOnlyProject: () => {},
  });

  const star = screen.getByLabelText("app:ctx.favoriteProject");
  assert.equal(star.querySelector("path").getAttribute("fill"), "none");
  fireEvent.click(star);
  assert.deepEqual(favorited, ["/work/payments"]);
});

test("动作按钮的点击不会顺带把文件夹折叠掉", () => {
  const toggled = [];
  renderLibrary({
    groupMode: "project",
    groups: [projectGroup()],
    favorites: [],
    onToggle: (key, kind) => toggled.push([key, kind]),
    onFavoriteProject: () => {},
    onOnlyProject: () => {},
  });

  fireEvent.click(screen.getByLabelText("app:ctx.favoriteProject"));
  assert.deepEqual(toggled, []);
});

test("进入项目范围后标题左侧出现返回按钮,Esc 在列表上也返回", () => {
  const backs = [];
  const { container } = render(
    <Pane collapsed={false} width={300} dragging={false} title="payments" count={54}
      query="" onQuery={() => {}} onOpenSearch={() => {}} onClearSearch={() => {}}
      onBack={() => backs.push("back")} backLabel="返回全部会话"
      listKey="library"><div /></Pane>,
  );

  fireEvent.click(screen.getByLabelText("返回全部会话"));
  assert.deepEqual(backs, ["back"]);

  fireEvent.keyDown(container.querySelector("[data-pane-scroll]"), { key: "Escape" });
  assert.deepEqual(backs, ["back", "back"]);
});

test("没有上一层范围时不画返回按钮", () => {
  render(
    <Pane collapsed={false} width={300} dragging={false} title="全部会话" count={128}
      query="" onQuery={() => {}} onOpenSearch={() => {}} onClearSearch={() => {}}
      listKey="library"><div /></Pane>,
  );
  assert.equal(document.querySelector("[data-pane-back]"), null);
});
