import type { RuntimeEngineMethod } from "../sessions/engine-store.js";
import { ProtocolError } from "../server/messages.js";
import type { FerryToolName, ToolRequestContext } from "./catalog.js";
import type { RuntimeEventBus } from "../runtime/event-bus.js";

export type ToolHandler = (
  name: FerryToolName,
  args: Record<string, unknown>,
  context: ToolRequestContext,
) => Promise<unknown>;

interface PendingRequest {
  sessionId: string;
  resolve(value: unknown): void;
  reject(error: Error): void;
  cleanup(): void;
}

const TOOL_DEADLINES_MS: Record<FerryToolName, number> = {
  session_search: 25_000,
  session_read: 25_000,
  usage: 25_000,
  migrate: 125_000,
  session_edit: 125_000,
  session_cleanup: 300_000,
  // 略大于 Rust 侧 24h 的挂起超时,让那边先触发、把 answered:false 作为结构化结果送回来
  ask_user: 86_400_000 + 30_000,
  agent_prompt: 400_000,
  // 略大于 Rust 侧 120s 的执行超时,让那边先触发、把进程组杀干净
  bash: 125_000,
};

interface RuntimeGatewayOptions {
  newId: () => string;
  events: RuntimeEventBus;
  emitToolRequest: (
    sessionId: string,
    runId: string,
    payload: Record<string, unknown>,
  ) => Promise<void>;
  toolHandler?: ToolHandler;
  toolDeadlinesMs?: Partial<Record<FerryToolName, number>>;
}

export class RuntimeGateway {
  private readonly pending = new Map<string, PendingRequest>();
  private readonly deadlines: Record<FerryToolName, number>;

  constructor(private readonly options: RuntimeGatewayOptions) {
    this.deadlines = {
      ...TOOL_DEADLINES_MS,
      ...options.toolDeadlinesMs,
    };
  }

  async invokeTool(
    name: FerryToolName,
    args: Record<string, unknown>,
    context: ToolRequestContext,
  ) {
    if (this.options.toolHandler) {
      return this.options.toolHandler(name, args, context);
    }
    const requestId = this.options.newId();
    let abortListener: (() => void) | undefined;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const result = new Promise<unknown>((resolve, reject) => {
      const cleanup = () => {
        if (abortListener) {
          context.signal?.removeEventListener("abort", abortListener);
        }
        if (timeout) clearTimeout(timeout);
      };
      this.pending.set(requestId, {
        sessionId: context.sessionId,
        resolve,
        reject,
        cleanup,
      });
      abortListener = () => {
        this.pending.delete(requestId);
        cleanup();
        reject(new Error("tool request aborted"));
      };
      if (context.signal?.aborted) {
        abortListener();
      } else {
        context.signal?.addEventListener("abort", abortListener, {
          once: true,
        });
        timeout = setTimeout(() => {
          if (!this.pending.delete(requestId)) return;
          cleanup();
          reject(new Error("tool gateway timed out"));
        }, this.deadlines[name]);
      }
    });
    if (!this.pending.has(requestId)) return result;
    try {
      await this.options.emitToolRequest(context.sessionId, context.runId, {
        request_id: requestId,
        tool_call_id: context.toolCallId,
        name,
        args,
        apply_policy: context.applyPolicy,
      });
    } catch (error) {
      const pending = this.pending.get(requestId);
      this.pending.delete(requestId);
      pending?.cleanup();
      pending?.reject(
        error instanceof Error ? error : new Error("tool request failed"),
      );
    }
    return result;
  }

  invokeEngine(
    method: RuntimeEngineMethod,
    params: Record<string, unknown>,
    sessionId: string,
  ) {
    const requestId = this.options.newId();
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const result = new Promise<unknown>((resolve, reject) => {
      const cleanup = () => {
        if (timeout) clearTimeout(timeout);
      };
      this.pending.set(requestId, {
        sessionId,
        resolve,
        reject,
        cleanup,
      });
      timeout = setTimeout(() => {
        if (!this.pending.delete(requestId)) return;
        cleanup();
        reject(new Error("engine gateway timed out"));
      }, 125_000);
    });
    this.options.events.emit(
      "engine.request",
      { request_id: requestId, method, params },
      sessionId,
      sessionId,
    );
    return result;
  }

  complete(requestId: string, sessionId: string, ok: boolean, value: unknown) {
    const pending = this.pending.get(requestId);
    if (!pending || pending.sessionId !== sessionId) {
      throw new ProtocolError("unknown_tool_request", "tool request not found");
    }
    this.pending.delete(requestId);
    pending.cleanup();
    if (ok) pending.resolve(value);
    else {
      pending.reject(
        new Error(
          typeof value === "string" ? value : "tool gateway rejected request",
        ),
      );
    }
    return { accepted: true };
  }
}
