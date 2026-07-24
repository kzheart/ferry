import { invoke } from "@tauri-apps/api/core";

import { throwEngineError } from "./errors.js";

async function transport(command, request) {
  try {
    return await invoke(command, { request });
  } catch (error) {
    throwEngineError(typeof error === "string" ? error : (error?.message || String(error)));
  }
}

async function callEngine(command, method, params) {
  const request = JSON.stringify({ method, params: params || {} });
  const raw = await transport(command, request);
  const response = JSON.parse(raw);
  if (!response.ok) throwEngineError(response.error);
  return response.result;
}

export const rpc = (method, params) => callEngine("engine_rpc", method, params);
export const trustedRpc = (method, params) =>
  callEngine("trusted_engine_rpc", method, params);

async function nativeOperation(command, args) {
  let raw;
  try {
    raw = await invoke(command, args);
  } catch (error) {
    throwEngineError(typeof error === "string" ? error : (error?.message || String(error)));
  }
  const response = JSON.parse(raw);
  if (!response.ok) throwEngineError(response.error);
  return response.result;
}

export const operationPlan = input =>
  nativeOperation("operation_plan", { input });

export const operationApply = planId =>
  nativeOperation("operation_apply", { planId });

export const operationStatus = planId =>
  nativeOperation("operation_status", { planId });

export const operationCancel = planId =>
  nativeOperation("operation_cancel", { planId });

// 启动描述符仍由引擎生成；终端偏好只决定原生层用哪个应用承载它。
export const openTerminal = (launch, terminalApp = "auto") =>
  invoke("open_terminal", { launch, terminalApp });

export const revealPath = path =>
  invoke("reveal_path", { path });

export const writeClipboardText = async text => {
  const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
  return writeText(String(text));
};

export const readClipboardText = async () => {
  const { readText } = await import("@tauri-apps/plugin-clipboard-manager");
  return readText();
};

// 原生菜单栏事件:handler 收到菜单项 id("settings"/"toggle-sidebar"/"rescan"),返回取消订阅函数
export const onMenu = async handler => {
  const { listen } = await import("@tauri-apps/api/event");
  return listen("menu", e => handler(e.payload));
};

// 窗口句柄需预加载:startDragging 必须在 mousedown 同步栈里调用才能抓住拖拽手势,
// 动态 import 的异步延迟会让手势过期,所以启动时先缓存句柄。
let _win = null;
export const preloadWindow = async () => {
  if (_win) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  _win = getCurrentWindow();
};

// 手动触发窗口拖拽:透明窗口下 Tauri 内建的 data-tauri-drag-region 会失效,改为主动调用
export const startWindowDrag = () => { _win?.startDragging?.(); };

// 双击标题栏切换最大化(macOS 惯例)
export const toggleWindowMaximize = () => { _win?.toggleMaximize?.(); };

// 窗口外观与应用主题同步(毛玻璃材质/红绿灯):theme 为 "light"|"dark",null 表示跟随系统
export const setWindowTheme = async theme => {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().setTheme(theme);
};
