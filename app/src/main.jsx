import React from "react";
import ReactDOM from "react-dom/client";
import "./app/architectureBoundary.js";
import { initI18n } from "./i18n/index.js";
import { hydrateToolsFromCache, loadTools } from "./api/contract/tools.js";
import { preloadBrowserCache } from "./features/browser/useBrowserData.js";
import App from "./app/App.jsx";
import "./app/style.css";

// 平台标记供 CSS 判断(macOS 下窗口透明走 vibrancy);
// 屏蔽 WebView 默认右键菜单,输入框与可选中文本放行(保留系统的复制/粘贴菜单)
if (navigator.userAgent.includes("Mac")) document.documentElement.dataset.platform = "mac";
window.addEventListener("contextmenu", e => {
  if (e.target instanceof Element &&
    e.target.closest('input, textarea, [contenteditable], .selectable')) return;
  e.preventDefault();
});

// 秒开:i18n 与 Agent 清单同步水合,数据缓存(IndexedDB,毫秒级)预读完成后立即挂载,
// 首帧即带上次数据,不闪加载态;引擎冷启动(release 下 PyInstaller 解压数秒)后台进行。
// 兜底 200ms:IndexedDB 异常卡住时也照常挂载,只是退回加载态
initI18n();
hydrateToolsFromCache();
Promise.race([
  preloadBrowserCache(),
  new Promise(resolve => setTimeout(resolve, 200)),
]).finally(() => {
  ReactDOM.createRoot(document.getElementById("root")).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
  // index.html 在 CSS 就绪前先落了一层底色防白闪,首帧后交还给样式表(macOS 下要透出毛玻璃)
  requestAnimationFrame(() => { document.documentElement.style.background = ""; });
});
loadTools();
