// 引擎结构化错误码 → 本地化文案。params 只含语义字段，句子在这里拼。
const MESSAGES = {
  "rpc.invalid_json": () => "请求格式错误",
  "rpc.unknown_method": p => `未知接口: ${p.method ?? ""}`,
  "rpc.missing_param": p => `缺少参数: ${p.param ?? ""}`,
  "tool.unknown": p => `未知工具: ${p.tool ?? ""}`,
  "tool.not_installed": p => `${p.tool ?? "工具"} 未安装`,
  "session.not_found": p => `找不到 ${p.tool ?? ""} 会话 ${p.ref ?? ""}`,
  "session.concurrent_modification": () => "会话在预览后已变化，请重新预览",
  "session.locator_stale": () => "定位符已失效，请刷新会话",
  "edit.operation_unsupported": p =>
    p.capability
      ? `${p.tool ?? ""} 不支持 ${p.capability}`
      : `${p.tool ?? ""} 不支持操作 ${p.operation ?? ""}${p.mode ? `（${p.mode}）` : ""}`,
  "edit.turn_out_of_range": p =>
    p.turn_count != null
      ? `轮次超界: 请求第 ${p.requested_turn} 轮，共 ${p.turn_count} 轮`
      : "轮次必须是正整数",
  "authoring.invalid_reply": () => "回复内容结构非法",
  "authoring.subagent_not_supported": () => "包含子 Agent spawn/task，authoring 已拒绝",
  "snapshot.not_found": () => "没有可用的快照",
  "snapshot.invalid_source": () => "快照来源非法，无法执行",
  "migration.tool_degraded": p => `工具 ${p.tool_name ?? ""} 已降级为叙述文本`,
  "migration.content_dropped": () => "部分内容在迁移中被丢弃",
  "migration.topology_mismatch": () => "迁移后的会话拓扑不一致",
  "probe.timeout": () => "探针超时",
  "probe.non_json_output": () => "探针输出无法解析",
  "probe.process_failed": p =>
    `探针进程失败${p.exit_code != null ? `（退出码 ${p.exit_code}）` : ""}`,
  "internal.unexpected": () => "引擎内部错误",
};

export class EngineError extends Error {
  constructor(payload) {
    const { code = "internal.unexpected", params = {} } = payload || {};
    const render = MESSAGES[code];
    super(render ? render(params) : `引擎错误: ${code}`);
    this.name = "EngineError";
    this.code = code;
    this.params = params;
    this.category = payload?.category;
    this.retryable = !!payload?.retryable;
  }
}

export function throwEngineError(error) {
  if (typeof error === "string") throw new Error(error || "引擎调用失败");
  throw new EngineError(error);
}
