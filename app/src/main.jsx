import React from "react";
import ReactDOM from "react-dom/client";
import "./app/architectureBoundary.js";
import { initI18n } from "./i18n/index.js";
import { loadTools } from "./api/contract/tools.js";
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

// 先初始化 i18n(同步,读 localStorage/navigator.language 决定语言),
// 再水合 Agent 清单(manifest 单一事实源),最后挂载应用
initI18n();
loadTools().finally(() => {
  ReactDOM.createRoot(document.getElementById("root")).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
});
