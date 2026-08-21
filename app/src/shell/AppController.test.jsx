// 整棵树的挂载冒烟。
//
// 单独测某一层时,Provider 是测试自己搭的,搭错了也测不出来;只有把主壳整个
// 挂起来,"某个 Context 忘了在真实树里注入"才会暴露。这类问题类型检查看不见,
// 也不会被任何单层用例发现,所以这条用例是 Context 化最主要的兜底。
//
// 断言一律走结构选择器而非文案:主壳挂载时会切到真实语言,cimode 的 key 回显
// 在这里不成立,而 data-* 标记不受文案与语言影响。
// 桌面 IPC 在 jsdom 里一律失败(见 vitest.setup.js),所以这里看的是取不到数据
// 时骨架是否仍然立得住,而不是白屏。
import { test, vi } from "vitest";
import assert from "node:assert/strict";
import { act, render } from "@testing-library/react";

// 桌面传输层整体替换掉:光挂载空壳看不见详情区,而 Context 缺注入恰恰只在
// 真正渲染出内容的那条路径上才会暴露。
const SESSION = {
  tool: "claude", id: "s1", title: "一次会话", ref: "fsr_claude_s1",
  dir: "/repo/ferry", mtime: "2026-01-01T00:00:00Z", size: 1024, msgs: 2,
};

// 特性开关:宿主侧的事实源在这里由用例决定。
const host = vi.hoisted(() => ({ builtinAgent: true }));

// 只替换取数据的入口,其余导出保持真实:传输层新增方法时这里不必跟着改。
vi.mock("../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  featuresList: async () => [{
    id: "builtin-agent", stage: "experimental", default: false,
    enabled: host.builtinAgent,
  }],
  featureSet: async (_id, enabled) => { host.builtinAgent = enabled; },
  engine: async (method) => {
    if (method === "scan") return { tools: { claude: 1 }, sessions: [SESSION] };
    if (method === "show") {
      return {
        messages: [
          { role: "user", text: "问题", n: 1 },
          { role: "assistant", text: "回答", n: 1 },
        ],
        turns: [{ n: 1 }],
        total: 2,
      };
    }
    if (method === "history" || method === "session_meta_list") return [];
    if (method === "pricing") return { prices: {} };
    if (method === "env") return { claude: true };
    return null;
  },
  runtime: async (method) => {
    if (method === "health") return { status: "ready" };
    if (method === "sessions.list") return [];
    if (method === "roles.list") {
      return [
        {
          id: "session-optimizer", name: "会话优化器", builtin: true,
          icon: "wand-sparkles", color: "violet", optimizer: true,
          tools: ["session_search", "session_read", "session_edit"],
        },
        // 有读写工具但没有 optimizer 标记:不该出现在优化器下拉里
        {
          id: "default", name: "通用助手", builtin: true,
          tools: ["session_search", "session_read", "session_edit"],
        },
      ];
    }
    if (method === "models.enabled") return [];
    return null;
  },
}));

import App from "./AppController.jsx";

// 没有资源栏的工作区
const WORKSPACES = [
  { key: "overview", pane: false },
  { key: "library", pane: true },
  { key: "history", pane: true },
  { key: "askferry", pane: true },
];

// 挂载期有若干条 IPC 在飞,让它们全部落地再断言。
async function mountApp() {
  let result;
  await act(async () => { result = render(<App />); });
  return result;
}

const railItem = (container, key) =>
  container.querySelector(`[data-rail-key="${key}"]`);

test("主壳能挂载,四个工作区的导航项都在", async () => {
  const { container } = await mountApp();

  assert.ok(container.querySelector('[data-ferry-win="1"]'));
  for (const { key } of WORKSPACES) {
    assert.ok(railItem(container, key), `缺少导航项 ${key}`);
  }
});

test("四个工作区逐一切过去都能渲染,资源栏按工作区出现或隐藏", async () => {
  const { container } = await mountApp();

  for (const { key, pane } of WORKSPACES) {
    await act(async () => { railItem(container, key).click(); });

    assert.ok(container.querySelector('[data-ferry-win="1"]'), `${key} 把主壳渲染没了`);
    assert.equal(
      Boolean(container.querySelector("[data-pane-scroll]")),
      pane,
      `${key} 的资源栏状态不对`,
    );
  }
});

test("回到起点仍然正常,说明工作区切换没有留下坏状态", async () => {
  const { container } = await mountApp();

  for (const key of ["library", "askferry", "history", "library"]) {
    await act(async () => { railItem(container, key).click(); });
  }
  assert.ok(container.querySelector("[data-pane-scroll]"));
});

test("内置 AI 助手关着时导航轨没有对话入口,其余工作区照旧", async () => {
  host.builtinAgent = false;
  try {
    const { container } = await mountApp();

    assert.equal(railItem(container, "askferry"), null, "对话入口不该出现");
    for (const key of ["overview", "library", "history"]) {
      assert.ok(railItem(container, key), `缺少导航项 ${key}`);
    }
  } finally {
    host.builtinAgent = true;
  }
});

test("优化入口默认不渲染:测试中功能需在设置里显式打开", async () => {
  const { container } = await mountApp();

  await act(async () => { railItem(container, "library").click(); });
  assert.ok(
    !container.querySelector('[data-optimize="session"]'),
    "开关默认关闭时不该出现优化入口",
  );
});

test("优化入口:可 rewrite 来源渲染分体魔法棒,角色下拉只列合格角色", async () => {
  localStorage.setItem("ferry-settings",
    JSON.stringify({ sessionOptimization: true }));
  const { container } = await mountApp();

  await act(async () => { railItem(container, "library").click(); });
  const entry = container.querySelector('[data-optimize="session"]');
  assert.ok(entry, "可 rewrite 来源没有渲染整段优化入口");

  // 点旁边的下拉箭头弹出角色列表:只列出具备 session_read+session_edit 的角色
  const caret = entry.parentElement.querySelector("button:nth-of-type(2)");
  assert.ok(caret, "魔法棒旁没有角色下拉箭头");
  await act(async () => { caret.click(); });
  assert.ok(
    container.textContent.includes("会话优化器"),
    "角色下拉没有列出内置优化器",
  );
  assert.ok(
    !container.textContent.includes("通用助手"),
    "未标记 optimizer 的角色不该出现在优化器下拉里",
  );
  // 不把开关留给后面的用例
  localStorage.removeItem("ferry-settings");
});

test("资料库选中会话后详情区能渲染,编辑面从 Context 取到", async () => {
  const { container } = await mountApp();

  await act(async () => { railItem(container, "library").click(); });

  // 首个会话会被自动选中并拉取详情;渲染到这一步说明 SessionDetail 及其
  // 依赖的 SessionEditingProvider 在真实树里是通的。
  assert.ok(container.textContent.includes("一次会话"), "详情区没有渲染出会话");
});
