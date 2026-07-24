// 组件测试跑在 Vite 的转换管线上:import.meta.glob、define、css 引入与生产构建
// 走同一条路径,测试里能渲染的组件就是构建产物里的那个组件。
import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config.js";

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: "jsdom",
      include: ["src/**/*.test.{js,jsx,ts,tsx}"],
      setupFiles: ["./vitest.setup.js"],
      restoreMocks: true,
    },
  }),
);
