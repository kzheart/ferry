import { invoke } from "@tauri-apps/api/core";

import type { FerryEventType } from "../../shared/contracts/generated/events.js";
import { isFerryEventType } from "../../shared/contracts/generated/events.js";
import type {
  PublicEngineMethod,
  TrustedUiEngineMethod,
} from "../../shared/contracts/generated/engine-methods.js";
import {
  FERRY_IPC_PROTOCOL,
  type FerryEvent,
  type IpcRequest,
  type IpcResponse,
} from "../../shared/contracts/generated/ipc.js";
import type {
  OperationInput,
  OperationPlan,
  OperationState,
} from "../../shared/contracts/operations.js";
import type { PublicRuntimeMethod } from "../../shared/contracts/generated/runtime-methods.js";
import { throwEngineError } from "./errors.js";

type DesktopParams = Record<string, unknown>;
type RuntimeEvent = FerryEvent & {
  type: FerryEventType;
};

let requestSequence = 1;

class RuntimeError extends Error {
  readonly code: string;

  constructor(code: string, message?: string) {
    super(message || code);
    this.name = "RuntimeError";
    this.code = code;
  }
}

async function invokeEngine<Result>(
  command: "engine_rpc" | "trusted_engine_rpc",
  method: string,
  params: DesktopParams = {},
): Promise<Result> {
  const request = JSON.stringify({ method, params });
  let raw: string;
  try {
    raw = await invoke<string>(command, { request });
  } catch (error) {
    throwEngineError(
      typeof error === "string"
        ? error
        : error instanceof Error
          ? error.message
          : String(error),
    );
  }
  const response = JSON.parse(raw) as IpcResponse<Result>;
  if (!response.ok) throwEngineError(response.error);
  return response.result;
}

async function invokeOperation<Result>(
  command:
    | "operation_plan"
    | "operation_apply"
    | "operation_status"
    | "operation_cancel",
  args: DesktopParams,
): Promise<Result> {
  let raw: string;
  try {
    raw = await invoke<string>(command, args);
  } catch (error) {
    throwEngineError(
      typeof error === "string"
        ? error
        : error instanceof Error
          ? error.message
          : String(error),
    );
  }
  const response = JSON.parse(raw) as IpcResponse<Result>;
  if (!response.ok) throwEngineError(response.error);
  return response.result;
}

export const engine = <Result = unknown>(
  method: PublicEngineMethod,
  params: DesktopParams = {},
) => invokeEngine<Result>("engine_rpc", method, params);

export const trustedEngine = <Result = unknown>(
  method: TrustedUiEngineMethod,
  params: DesktopParams = {},
) => invokeEngine<Result>("trusted_engine_rpc", method, params);

export async function runtime<Result = unknown>(
  method: PublicRuntimeMethod,
  params: DesktopParams = {},
): Promise<Result> {
  const request: IpcRequest<PublicRuntimeMethod> = {
    protocol: FERRY_IPC_PROTOCOL,
    id: `ui_${Date.now().toString(36)}_${requestSequence++}`,
    method,
    params,
  };
  let raw: string;
  try {
    raw = await invoke<string>("agent_command", {
      request: JSON.stringify(request),
    });
  } catch (error) {
    throw new RuntimeError("agent_unavailable", String(error));
  }
  const response = JSON.parse(raw) as IpcResponse<Result>;
  if (!response.ok) {
    throw new RuntimeError(
      response.error.code || "agent_error",
      String(response.error.params?.message || response.error.message || ""),
    );
  }
  return response.result;
}

export async function onRuntimeEvent(
  handler: (event: RuntimeEvent) => void,
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<FerryEvent>("ferry-runtime-event", event => {
    if (isFerryEventType(event.payload?.type)) {
      handler(event.payload as RuntimeEvent);
    }
  });
}

/** 引擎主动推送(如 sessions.changed 增量):独立于 Runtime 事件通道。 */
export async function onEngineEvent(
  handler: (event: RuntimeEvent) => void,
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<FerryEvent>("ferry-engine-event", event => {
    if (isFerryEventType(event.payload?.type)) {
      handler(event.payload as RuntimeEvent);
    }
  });
}

export const operationPlan = (input: OperationInput) =>
  invokeOperation<OperationPlan>("operation_plan", { input });

export const operationApply = (planId: string) =>
  invokeOperation<OperationState>("operation_apply", { planId });

export const operationStatus = (planId: string) =>
  invokeOperation<OperationState>("operation_status", { planId });

export const operationCancel = (planId: string) =>
  invokeOperation<OperationState>("operation_cancel", { planId });

export const openTerminal = (
  launch: DesktopParams,
  terminalApp = "auto",
) => invoke("open_terminal", { launch, terminalApp });

export const revealPath = (path: string) => invoke("reveal_path", { path });

/** 角色配置导出:系统保存对话框在 Rust 侧落盘,返回路径;取消返回 null。 */
export const exportRolesFile = (fileName: string, contents: string) =>
  invoke<string | null>("export_roles_file", { fileName, contents });

/** 角色配置导入:系统选择对话框在 Rust 侧读回文本;取消返回 null。 */
export const importRolesFile = () =>
  invoke<string | null>("import_roles_file");

/** bash 提案的批准:plan_id 带 shl_ 前缀,命令在 Rust 侧执行,不经 Engine。 */
export const shellApply = (planId: string) =>
  invoke<Record<string, unknown>>("bash_apply", { planId });

/** ask_user 的选择卡应答:请求由 Rust 宿主挂起,页面只提交结构化答案。 */
export const choiceRespond = (
  sessionId: string,
  requestId: string,
  answer: Record<string, unknown>,
) => invoke<void>("choice_respond", { sessionId, requestId, answer });

/** 技能目录选择:路径由系统对话框产生,webview 不能指定任意路径;取消返回 null。 */
export const pickSkillDirectory = () =>
  invoke<string | null>("pick_skill_directory");

export const writeClipboardText = async (text: unknown) => {
  const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
  return writeText(String(text));
};

export const readClipboardText = async () => {
  const { readText } = await import("@tauri-apps/plugin-clipboard-manager");
  return readText();
};

export const onMenu = async (handler: (payload: unknown) => void) => {
  const { listen } = await import("@tauri-apps/api/event");
  return listen("menu", event => handler(event.payload));
};

interface DesktopWindow {
  startDragging(): Promise<void>;
  toggleMaximize(): Promise<void>;
}

let currentWindow: DesktopWindow | null = null;

export const preloadWindow = async () => {
  if (currentWindow) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  currentWindow = getCurrentWindow();
};

export const startWindowDrag = () => {
  void currentWindow?.startDragging();
};

export const toggleWindowMaximize = () => {
  void currentWindow?.toggleMaximize();
};

export const setWindowTheme = async (
  theme: "light" | "dark" | null,
) => {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().setTheme(theme);
};
