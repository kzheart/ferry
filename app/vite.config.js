import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const appDir = dirname(fileURLToPath(import.meta.url));
// 版本单一来源:app/package.json(src-tauri/tauri.conf.json 的 version 也指向它)
const pkg = JSON.parse(readFileSync(resolve(appDir, "package.json"), "utf8"));

export default defineConfig({
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // Windows 上 Vite 默认会盯上 src-tauri/target 里的 .exe，触发 EBUSY 直接退出。
      ignored: ["**/src-tauri/**"],
    },
  },
});
