// 引擎结构化事件 → 本地化文案。旧持久化数据是纯字符串，原样透传。
const EVENTS = {
  "migration.reasoning_metadata_dropped": p =>
    `思考过程降级为纯文本（丢弃 ${p.metadata_kind ?? "元数据"}）`,
  "migration.reasoning_dropped": () => "思考过程无可见正文，已丢弃",
  "migration.unknown_block_dropped": p => `未知内容块类型丢弃: ${p.kind ?? ""}`,
  "migration.apply_patch_unparsed": () => "apply_patch 无法解析出文件变更，已降级",
  "migration.tool_degraded": p => `工具 ${p.tool_name ?? ""} 降级为叙述文本`,
  "migration.tool_dropped": p => `工具 ${p.tool_name ?? ""} 将被丢弃`,
  "migration.fork_parent_fallback": () => "分支父节点无法精确映射，回退到主对话末尾",
  "migration.truncated": p =>
    `按迁移范围截断: 丢弃第 ${p.max_turn} 轮之后的 ${p.dropped} 条消息`,
  "migration.children_not_migrated": p => `截断范围外的 ${p.count} 个子会话未迁移`,
  "session.orphan_tool_result": p => `孤儿工具结果: ${p.call_id ?? ""}`,
  "session.unpaired_tool_use": p => `未配对工具调用 ${p.tool_name ?? ""}，按无输出处理`,
  "session.subagent_unlinked": p => `子代理 ${p.child_id ?? ""} 无法精确关联，按根子节点保留`,
  "session.child_foreign_ignored": p => `子会话 ${p.child_id ?? ""} 不属于当前父会话，已忽略`,
  "session.child_parent_conflict": p => `子会话 ${p.child_id ?? ""} 的父指向冲突，已忽略`,
  "codex.thread_unregistered": p => `线程 ${p.session_id ?? ""} 无注册行，副本将由 Codex 扫描发现`,
  "codex.thread_edge_unregistered": p =>
    `线程边 ${p.parent ?? ""}→${p.child ?? ""} 未注册，端点缺少注册行`,
  "edit.reply_replaced": p => `替换第 ${p.turn} 轮 AI 回复，共 ${p.items} 个 item`,
  "edit.turn_deleted": p =>
    `删除第 ${p.turn} 轮${p.pruned_children ? `，同时移除 ${p.pruned_children} 个子会话` : ""}`,
  "edit.message_rewritten": () => "改写 1 条消息",
};

const SNAPSHOT_REASONS = {
  "snapshot.manual": "手动快照",
  "snapshot.before_delete": "删除前自动",
  "snapshot.before_edit": "会话编辑前自动",
  "snapshot.before_restore_guard": "还原前保护",
};

export function renderEvent(e) {
  if (e == null) return "";
  if (typeof e === "string") return e;
  const render = EVENTS[e.code];
  return render ? render(e.params || {}) : e.code || JSON.stringify(e);
}

export const renderEvents = list => (list || []).map(renderEvent);

const PROBE_CODES = {
  "probe.timeout": () => "探针超时",
  "probe.non_json_output": () => "探针输出无法解析",
  "probe.process_failed": p =>
    `探针进程失败${p.exit_code != null ? `（退出码 ${p.exit_code}）` : ""}`,
  "probe.structure_invalid": () => "结构验证未通过",
};

const ISOLATION_KINDS = {
  shadow_session: "影子会话",
  shadow_copy: "影子副本",
  temp_home: "临时环境",
};

// probe 报告双读：新结构 {status, code, diagnostic} 或旧历史 {ok, detail}
export const probeFailed = p =>
  !!p && (p.status === "failed" || p.ok === false);

export function probeText(p) {
  if (!p) return "";
  if (p.detail != null) return p.detail;
  const parts = [];
  if (p.isolation) {
    const kind = ISOLATION_KINDS[p.isolation.kind] || p.isolation.kind;
    parts.push(`(${kind} ${p.isolation.id ?? ""} 已探测并清理)`);
  }
  if (p.code) parts.push((PROBE_CODES[p.code] || (() => p.code))(p.params || {}));
  const d = p.diagnostic || {};
  if (d.stdout) parts.push(d.stdout);
  if (d.stderr) parts.push(d.stderr);
  if (d.truncated) parts.push("…(输出已截断)");
  return parts.filter(Boolean).join("\n");
}

export function renderSnapshotReason(snapshot) {
  return SNAPSHOT_REASONS[snapshot?.reason_code] ||
    snapshot?.legacy_reason || snapshot?.reason || "会话编辑前自动";
}
