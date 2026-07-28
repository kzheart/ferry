// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const FERRY_IPC_PROTOCOL = "ferry-ipc/1" as const;
export const FERRY_CONTRACT_HASH =
  "sha256:9da94e3db62ad211ccc29c921e0ff8b45108dfddd00600378f7e1ea3d865fd9c" as const;

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
