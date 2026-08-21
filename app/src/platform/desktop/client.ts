import { invoke } from "@tauri-apps/api/core";

import type { FerryEventType } from "../../shared/contracts/generated/events.js";
import { isFerryEventType } from "../../shared/contracts/generated/events.js";
import type { UiEngineMethod } from "../../shared/contracts/generated/engine-methods.js";
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
import type {
  FeatureId,
  FeatureStage,
} from "../../shared/contracts/generated/features.js";
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
  command: "engine_rpc",
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
  method: UiEngineMethod,
  params: DesktopParams = {},
) => invokeEngine<Result>("engine_rpc", method, params);

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
    // 宿主的拒绝可能是结构化的({code, message},如特性开关关着时的
    // feature.disabled),其余仍是纯文本。
    const structured = error as { code?: unknown; message?: unknown };
    if (structured && typeof structured.code === "string") {
      throw new RuntimeError(structured.code, String(structured.message ?? ""));
    }
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

export interface CliStatus {
  supported: boolean;
  unsupported_reason: string | null;
  link_path: string | null;
  installed: boolean;
  link_target: string | null;
  points_to_current_engine: boolean;
  engine_path: string | null;
  on_path: boolean;
}

export interface SkillTargetStatus {
  id: string;
  display_name: string;
  path: string;
  installed: boolean;
  installed_version: string | null;
  via_shared: boolean;
}

export interface IntegrationStatus {
  cli: CliStatus;
  skills: SkillTargetStatus[];
  bundled_version: string | null;
}

export interface EngineServiceStatus {
  state: "app-shared" | "daemon" | "stopped";
  pid: number | null;
  socket: string | null;
  socket_ready: boolean;
  version: string | null;
}

/** Agent 集成状态:CLI 入口与各 skill 目标的检测结果,路径全部由宿主计算。 */
export const integrationStatus = () =>
  invoke<IntegrationStatus>("integration_status");

/** 创建/重建 `~/.local/bin/ferry`,指向本 App 的引擎二进制。 */
export const cliInstall = () => invoke<void>("cli_install");

/** 移除 CLI 入口;宿主只删确实指向 Ferry 引擎的链接。 */
export const cliUninstall = () => invoke<void>("cli_uninstall");

/** 把打包的 Ferry skill 装进固定目标表里的某个目录;targetId 来自 integrationStatus。 */
export const skillInstall = (targetId: string) =>
  invoke<void>("skill_install", { targetId });

export const skillUninstall = (targetId: string) =>
  invoke<void>("skill_uninstall", { targetId });

/** 自定义目录安装:path 必须是 pickSkillDirectory 返回的目录,宿主再校验一次。 */
export const skillInstallCustom = (path: string) =>
  invoke<string>("skill_install_custom", { path });

/** 引擎服务状态:只读锁文件与 socket,不做任何进程操作。 */
export const engineServiceStatus = () =>
  invoke<EngineServiceStatus>("engine_service_status");

/** 「允许 CLI 共享 App 引擎」的当前值。事实源是宿主的配置文件,不是这里的状态。 */
export const getEngineShare = () => invoke<boolean>("get_engine_share");

/** 改开关。只落盘;sidecar 只在启动时决定是否监听 socket,所以下次启动 App 才生效。 */
export const setEngineShare = (enabled: boolean) =>
  invoke<void>("set_engine_share", { enabled });

/** 一个特性开关的契约形态 + 这台机器上的当前值。 */
export interface FeatureState {
  id: FeatureId;
  stage: FeatureStage;
  default: boolean;
  enabled: boolean;
}

/**
 * 全部特性开关的当前状态。事实源是宿主的配置文件:界面按它决定入口显不显示,而
 * 真正拦住能力的那道门在宿主侧自己回读同一份文件,不信 WebView 传来的任何东西。
 */
export const featuresList = () => invoke<FeatureState[]>("features_list");

/** 改一个特性。只落盘;门是每次请求回读的,所以下一次调用就按新值判。 */
export const featureSet = (id: FeatureId, enabled: boolean) =>
  invoke<void>("feature_set", { id, enabled });

/** 停止 CLI 拉起的独立 daemon。失败以 {@link DaemonStopError} 抛出。 */
export const engineDaemonStop = () => invoke<void>("engine_daemon_stop");

/** 宿主给的结构化失败:code 稳定可分支,message 兜底展示。 */
export interface DaemonStopError {
  /** app_mode = 对面是 App 自己的引擎,只能退出 App 来释放。 */
  code: "unsupported" | "unavailable" | "app_mode" | "refused" | "timeout";
  message: string;
}

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
