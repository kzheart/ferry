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

afterEach(cleanup);
