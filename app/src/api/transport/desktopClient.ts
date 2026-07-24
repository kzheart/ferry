import { invoke } from "@tauri-apps/api/core";

import type { FerryEventType } from "../contract/generated/events.js";
import { isFerryEventType } from "../contract/generated/events.js";
import type {
  PublicEngineMethod,
  TrustedUiEngineMethod,
} from "../contract/generated/engine-methods.js";
import {
  FERRY_IPC_PROTOCOL,
  type FerryEvent,
  type IpcRequest,
  type IpcResponse,
} from "../contract/generated/ipc.js";
import type {
  OperationInput,
  OperationPlan,
  OperationState,
} from "../../features/operations/operationController.js";
import type { PublicRuntimeMethod } from "../contract/generated/runtime-methods.js";
import { throwEngineError } from "./errors.js";

export type DesktopParams = Record<string, unknown>;
export type RuntimeEvent = FerryEvent & {
  type: FerryEventType;
};

let requestSequence = 1;

export class RuntimeError extends Error {
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
