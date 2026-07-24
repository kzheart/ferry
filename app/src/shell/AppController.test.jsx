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
  tool: "claude", id: "s1", title: "一次会话",
  dir: "/repo/ferry", mtime: "2026-01-01T00:00:00Z", size: 1024, msgs: 2,
};

// 只替换取数据的入口,其余导出保持真实:传输层新增方法时这里不必跟着改。
vi.mock("../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
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

test("资料库选中会话后详情区能渲染,编辑面从 Context 取到", async () => {
  const { container } = await mountApp();

  await act(async () => { railItem(container, "library").click(); });

  // 首个会话会被自动选中并拉取详情;渲染到这一步说明 SessionDetail 及其
  // 依赖的 SessionEditingProvider 在真实树里是通的。
  assert.ok(container.textContent.includes("一次会话"), "详情区没有渲染出会话");
});
