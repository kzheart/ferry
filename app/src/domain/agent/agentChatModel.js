// Agent 事件 → 聊天时间线的纯归约:实时事件与 events.replay 共用同一入口,
// 带 seq 的事件按序去重,重载后回放能得到一致的消息与工具状态
export const emptyLog = () => ({
  items: [],
  latestSeq: 0,
  status: "idle",
  runId: null,
  provider: null,
  model: null,
});

// 工具权限等级:审批卡与工具行徽章按这里着色
export const TOOL_LEVEL = {
  session_search: "read",
  session_read: "read",
  usage: "read",
  ferry_preview_migration: "preview",
  ferry_preview_edit: "preview",
  ferry_propose_migration: "mutate",
  ferry_propose_edit: "mutate",
  ferry_propose_metadata_change: "mutate",
};

const sealAssistant = items => {
  const last = items[items.length - 1];
  if (last?.kind === "assistant" && last.streaming) {
    items[items.length - 1] = { ...last, streaming: false };
  }
};

const endRun = (log, items) => {
  sealAssistant(items);
  log.status = "idle";
  log.runId = null;
};

export function applyEvent(log, ev) {
  if (typeof ev.seq === "number") {
    if (ev.seq <= log.latestSeq) return log;
    log = { ...log, latestSeq: ev.seq };
  } else {
    log = { ...log };
  }
  const items = (log.items = [...log.items]);
  const p = ev.payload || {};
  switch (ev.type) {
    case "session.created":
    case "session.model_changed":
      log.provider = p.provider_id;
      log.model = p.model_id;
      break;
    case "run.started":
      items.push({ kind: "user", text: p.prompt ?? "", imageCount: p.image_count || 0 });
      log.status = "running";
      log.runId = ev.run_id;
      break;
    case "user.message":
      items.push({ kind: "user", text: p.text ?? "", sub: p.kind });
      break;
    case "content.delta": {
      const last = items[items.length - 1];
      if (last?.kind === "assistant" && last.streaming) {
        items[items.length - 1] = { ...last, text: last.text + (p.delta || "") };
      } else {
        items.push({ kind: "assistant", text: p.delta || "", streaming: true, runId: ev.run_id });
      }
      break;
    }
    case "tool.started":
      sealAssistant(items);
      items.push({ kind: "tool", callId: p.tool_call_id, name: p.name, args: p.args,
        status: "running", startedAt: ev.timestamp });
      break;
    case "tool.completed": {
      const i = items.findLastIndex(it => it.kind === "tool" && it.callId === p.tool_call_id);
      if (i >= 0) items[i] = { ...items[i], status: p.is_error ? "error" : "ok",
        endedAt: ev.timestamp, result: p.result };
      break;
    }
    // Rust 可信边界补发,无 seq,不进事件日志;审批状态由前端本地推进
    case "operation.proposed":
      items.push({ kind: "approval", tool: p.tool, operation: p.operation || {},
        runId: ev.run_id, status: "pending" });
      break;
    case "operation.applied":
      items.push({ kind: "approval", tool: p.tool, operation: p.operation || {},
        runId: ev.run_id, status: "applied", result: p.result, auto: !!p.auto });
      break;
    case "operation.failed":
      items.push({ kind: "approval", tool: p.tool, operation: p.operation || {},
        runId: ev.run_id, status: "failed", error: p.error, auto: !!p.auto });
      break;
    case "run.completed":
      endRun(log, items);
      break;
    case "run.failed":
      endRun(log, items);
      items.push({ kind: "status", type: ev.type, message: p.message });
      break;
    case "run.cancelled":
    case "run.interrupted":
      endRun(log, items);
      items.push({ kind: "status", type: ev.type });
      break;
  }
  return log;
}

// 本地推进审批卡状态(applying/applied/failed/dismissed)
export function patchApproval(log, operationId, patch) {
  const i = log.items.findLastIndex(
    it => it.kind === "approval" && it.operation?.operation_id === operationId);
  if (i < 0) return log;
  const items = [...log.items];
  items[i] = { ...items[i], ...patch };
  return { ...log, items };
}

// 对话标题:第一条用户消息的首行
export function titleOf(log) {
  const first = log?.items.find(it => it.kind === "user" && it.text);
  return first ? first.text.split("\n")[0].slice(0, 60) : null;
}
