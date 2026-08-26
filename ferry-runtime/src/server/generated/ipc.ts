// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const FERRY_IPC_PROTOCOL = "ferry-ipc/1" as const;
export const FERRY_CONTRACT_HASH =
  "sha256:2a61a534e1e27e48edde9811d884da8a11ef80875d5f3862939619eab0bd79d2" as const;

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
