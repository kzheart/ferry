// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const FERRY_IPC_PROTOCOL = "ferry-ipc/1" as const;
export const FERRY_CONTRACT_HASH = "sha256:551ed5a7c3919f0364667d31ea83298af5ce3d2abe0cf3c4c97e58b00d2978a1" as const;
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
