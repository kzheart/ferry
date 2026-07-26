// 会话优化入口的纯函数与常量:purpose 与默认角色 id 必须与 ferry-runtime 契约一致
export const SESSION_OPTIMIZATION_PURPOSE = "session-optimization";
export const SESSION_OPTIMIZER_ROLE_ID = "session-optimizer";

/** 任何非法输入都归一为 general:newChat 可能被当成事件回调直接传 event 对象。 */
export function normalizeSessionPurpose(value) {
  return value === SESSION_OPTIMIZATION_PURPOSE
    ? SESSION_OPTIMIZATION_PURPOSE
    : "general";
}

/**
 * 生成优化会话的可编辑首条草稿。target.turn 缺省表示优化整段会话;
 * 草稿只预填不自动发送,用户可以在发送前继续修改目标或措辞。
 */
export function buildSessionOptimizationDraft(target) {
  const turn = Number.isInteger(target?.turn) && target.turn >= 1
    ? target.turn
    : null;
  if (turn !== null) {
    return `请优化附件会话中第 ${turn} 轮的用户提问:先用 session_read 读取该消息,`
      + "给出忠实于原意、更清晰完整的改写候选(preview),等我确认后再写回。";
  }
  return "请通读附件会话,找出表述不清、缺少上下文或容易被误解的用户提问,"
    + "先用 session_read 读取原文,再给出整体改写候选(preview),等我确认后再写回。";
}
