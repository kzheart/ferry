//! Short-lived native JSON-RPC transport. No prompt or approval execution.
import type { ChildProcessWithoutNullStreams } from "node:child_process";
import spawn from "cross-spawn";
import { ProtocolError } from "../server/messages.js";

export class NativeRpc {
  private readonly child: ChildProcessWithoutNullStreams;
  private sequence = 0;
  private buffer = "";
  private stopped = false;
  private readonly pending = new Map<
    number,
    {
      resolve: (value: any) => void;
      reject: (e: Error) => void;
      timer: NodeJS.Timeout;
    }
  >();
  constructor(
    executable: string,
    args: string[],
    cwd: string,
    private readonly timeoutMs = 30_000,
  ) {
    this.child = spawn(executable, args, {
      cwd,
      stdio: "pipe",
      windowsHide: true,
    }) as ChildProcessWithoutNullStreams;
    this.child.stderr.resume();
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (chunk: string) => {
      this.buffer += chunk;
      if (this.buffer.length > 32 * 1024 * 1024) {
        this.fail(new Error("Native RPC response too large"));
        return;
      }
      let end: number;
      while ((end = this.buffer.indexOf("\n")) >= 0) {
        const line = this.buffer.slice(0, end);
        this.buffer = this.buffer.slice(end + 1);
        let value: any;
        try {
          value = JSON.parse(line);
        } catch {
          this.fail(new Error("Invalid native RPC JSON"));
          return;
        }
        // Refuse server-initiated tool/approval requests. Forking must not run the agent.
        if (value.method && value.id !== undefined) {
          this.notifyResponse({
            jsonrpc: "2.0",
            id: value.id,
            error: {
              code: -32601,
              message: "Ferry fork client does not execute tools",
            },
          });
          continue;
        }
        const call = this.pending.get(value.id);
        if (!call) continue;
        clearTimeout(call.timer);
        this.pending.delete(value.id);
        if (value.error)
          call.reject(
            new ProtocolError(
              "invalid_params",
              String(value.error.message ?? "Native fork rejected"),
            ),
          );
        else call.resolve(value.result);
      }
    });
    this.child.on("error", (e) => this.fail(e));
    this.child.on("exit", () =>
      this.fail(new Error("Native RPC exited before responding")),
    );
    this.child.stdin.on("error", (e) => this.fail(e));
  }
  private notifyResponse(value: unknown) {
    if (!this.stopped) this.child.stdin.write(`${JSON.stringify(value)}\n`);
  }
  notify(method: string, params?: unknown) {
    this.notifyResponse({ jsonrpc: "2.0", method, params });
  }
  call(method: string, params: unknown): Promise<any> {
    if (this.stopped) return Promise.reject(new Error("Native RPC closed"));
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(
        () =>
          this.fail(
            new Error(
              "Native fork request timed out; its outcome may be unknown",
            ),
          ),
        this.timeoutMs,
      );
      this.pending.set(id, { resolve, reject, timer });
      this.notifyResponse({ jsonrpc: "2.0", id, method, params });
    });
  }
  private fail(error: Error) {
    for (const call of this.pending.values()) {
      clearTimeout(call.timer);
      call.reject(error);
    }
    this.pending.clear();
    this.close();
  }
  close() {
    if (this.stopped) return;
    this.stopped = true;
    this.child.stdin.end();
    this.child.kill();
    const timer = setTimeout(() => this.child.kill("SIGKILL"), 1000);
    timer.unref();
    this.child.once("exit", () => clearTimeout(timer));
  }
}
