// 会话优化入口的纯函数与常量:purpose 与默认角色 id 必须与 ferry-runtime 契约一致
export const SESSION_OPTIMIZATION_PURPOSE = "session-optimization";
export const SESSION_OPTIMIZER_ROLE_ID = "session-optimizer";

/** 任何非法输入都归一为 general:newChat 可能被当成事件回调直接传 event 对象。 */
export function normalizeSessionPurpose(value) {
  return value === SESSION_OPTIMIZATION_PURPOSE
    ? SESSION_OPTIMIZATION_PURPOSE
    : "general";
}

