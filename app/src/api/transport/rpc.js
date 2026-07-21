import { invoke } from "@tauri-apps/api/core";

import { throwEngineError } from "./errors.js";

const inTauri = () => !!window.__TAURI_INTERNALS__;

export async function rpc(method, params) {
  const request = JSON.stringify({ method, params: params || {} });
  const raw = inTauri()
    ? await invoke("engine_rpc", { request })
    : await (await fetch("/api/rpc", { method: "POST", body: request })).text();
  const response = JSON.parse(raw);
  if (!response.ok) throwEngineError(response.error);
  return response.result;
}

// The desktop command currently accepts the existing launch DTO as-is.
export const openTerminal = launch =>
  inTauri() ? invoke("open_terminal", { launch }) : Promise.resolve();

export const revealPath = path =>
  inTauri() ? invoke("reveal_path", { path }) : Promise.resolve();

export const canReveal = () => inTauri();

// 原生菜单栏事件:handler 收到菜单项 id("settings"/"toggle-sidebar"/"rescan"),返回取消订阅函数
export const onMenu = async handler => {
  if (!inTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen("menu", e => handler(e.payload));
};

// 窗口句柄需预加载:startDragging 必须在 mousedown 同步栈里调用才能抓住拖拽手势,
// 动态 import 的异步延迟会让手势过期,所以启动时先缓存句柄。
let _win = null;
export const preloadWindow = async () => {
  if (!inTauri() || _win) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  _win = getCurrentWindow();
};

// 手动触发窗口拖拽:透明窗口下 Tauri 内建的 data-tauri-drag-region 会失效,改为主动调用
export const startWindowDrag = () => { _win?.startDragging?.(); };

// 双击标题栏切换最大化(macOS 惯例)
export const toggleWindowMaximize = () => { _win?.toggleMaximize?.(); };

// 窗口外观与应用主题同步(毛玻璃材质/红绿灯):theme 为 "light"|"dark",null 表示跟随系统
export const setWindowTheme = async theme => {
  if (!inTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().setTheme(theme);
};
