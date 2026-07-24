// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const FERRY_IPC_PROTOCOL = "ferry-ipc/1" as const;
export const FERRY_CONTRACT_HASH =
  "sha256:a03e9e616707badc0e4c8b995907ec6e147cff7af405413e11e00c4c47d5aa80" as const;

export interface IpcRequest<Method extends string = string> {
  protocol: typeof FERRY_IPC_PROTOCOL;
  id: string;
  method: Method;
  params: Record<string, unknown>;
}

export interface IpcError {
  code: string;
  category: string;
  retryable: boolean;
  params: Record<string, unknown>;
}

export interface IpcSuccessResponse {
  protocol: typeof FERRY_IPC_PROTOCOL;
  id: string;
  ok: true;
  result: unknown;
}

export interface IpcFailureResponse {
  protocol: typeof FERRY_IPC_PROTOCOL;
  id: string;
  ok: false;
  error: IpcError;
}

export type IpcResponse = IpcSuccessResponse | IpcFailureResponse;

export interface IpcEvent {
  protocol: typeof FERRY_IPC_PROTOCOL;
  type: string;
  correlation_id?: string;
  context?: Record<string, unknown>;
  payload: Record<string, unknown>;
}
