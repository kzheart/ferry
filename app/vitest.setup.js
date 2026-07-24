// 组件测试的全局装置。
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";

// cimode 让 t("ns:key") 原样返回 key:断言绑定在渲染结构上,改文案不会把测试打红。
i18n.use(initReactI18next).init({
  lng: "cimode",
  appendNamespaceToCIMode: true,
  resources: {},
  interpolation: { escapeValue: false },
});

// jsdom 没有 ResizeObserver;列表虚拟化靠它测量视口。测试里布局恒为 0,
// 用空实现即可——需要断言可视区行数的用例应该直接测虚拟化模型,不走渲染。
if (!globalThis.ResizeObserver) {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

// jsdom 不做布局,滚动相关的 API 一律缺失。组件里"滚到某处"是纯副作用,
// 空实现不影响任何断言,却能让调用它的组件正常渲染。
for (const method of ["scrollTo", "scrollBy", "scrollIntoView"]) {
  if (!Element.prototype[method]) Element.prototype[method] = () => {};
}

// Tauri 的桥在浏览器里由宿主注入,jsdom 里没有。给一个会拒绝的桩:调用方本来
// 就要处理 IPC 失败,于是整棵树能挂载起来,而不是在第一次 invoke 时崩掉。
// 想断言某条 IPC 行为的用例应该自己 mock platform/desktop/client,不要依赖这里。
globalThis.__TAURI_INTERNALS__ = {
  transformCallback: () => 0,
  // 事件订阅要成功,否则挂载期的 listen 会变成没人接的 rejection;真正取数据的
  // 命令一律失败,让组件走它自己的错误分支。
  invoke: async (command) => {
    if (String(command).startsWith("plugin:event|")) return 0;
    throw new Error("桌面 IPC 在测试环境不可用");
  },
  convertFileSrc: (path) => path,
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
};
globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };

afterEach(cleanup);
