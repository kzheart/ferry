// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const FERRY_IPC_PROTOCOL = "ferry-ipc/1" as const;
export const FERRY_CONTRACT_HASH = "sha256:335daa3550349d6cd0de1af9c52cbf36bc5321c75526f954b284ddbe72f5cc49" as const;
export interface IpcRequest<Method extends string = string> {
  protocol: typeof FERRY_IPC_PROTOCOL;
  id: string;
  method: Method;
  params: Record<string, unknown>;
}
export interface IpcError {
  code: string;
  category?: string;
  retryable?: boolean;
  params?: Record<string, unknown>;
  message?: string;
}
export type IpcResponse<Result = unknown> =
  | { ok: true; result: Result }
  | { ok: false; error: IpcError };
export interface FerryEvent {
  type: string;
  payload?: Record<string, unknown>;
  [key: string]: unknown;
}
