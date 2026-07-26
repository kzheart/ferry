// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const FERRY_IPC_PROTOCOL = "ferry-ipc/1" as const;
export const FERRY_CONTRACT_HASH = "sha256:448fba832102442444e9011dc7614fb5a0026da430be950986f1b8842a2f67fd" as const;
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
