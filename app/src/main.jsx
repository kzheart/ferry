import React from "react";
import ReactDOM from "react-dom/client";
import "./app/architectureBoundary.js";
import { loadTools } from "./api/contract/tools.js";
import App from "./app/App.jsx";
import "./app/style.css";

// 先水合 Agent 清单(manifest 单一事实源),再挂载应用
loadTools().finally(() => {
  ReactDOM.createRoot(document.getElementById("root")).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
});
