import i18n from "../../i18n/index.js";

const t = (key, params) => i18n.t(key, params);

// 旧持久化数据可能是纯字符串,原样透传;新数据按 code 走 i18n。
export function renderEvent(e) {
  if (e == null) return "";
  if (typeof e === "string") return e;
  const p = e.params || {};
  const code = e.code;
  if (!code) return JSON.stringify(e);

  // events.json 用扁平化的 code→key 映射,这里按 code 分支拼 key
  // 带参数变体的 key 在各自 branch 里处理
  switch (code) {
    case "migration.reasoning_metadata_dropped":
      return t("events:migration.reasoning_metadata_dropped", {
        metadata_kind_label: p.metadata_kind ?? t("events:migration.metadata_kind_default"),
      });
    case "migration.reasoning_dropped":
      return t("events:migration.reasoning_dropped");
    case "migration.unknown_block_dropped":
      return t("events:migration.unknown_block_dropped", { kind: p.kind ?? "" });
    case "migration.apply_patch_unparsed":
      return t("events:migration.apply_patch_unparsed");
    case "migration.tool_degraded":
      return t("events:migration.tool_degraded", { tool_name: p.tool_name ?? "" });
    case "migration.tool_dropped":
      return t("events:migration.tool_dropped", { tool_name: p.tool_name ?? "" });
    case "migration.fork_parent_fallback":
      return t("events:migration.fork_parent_fallback");
    case "migration.truncated":
      return t("events:migration.truncated", { max_turn: p.max_turn, dropped: p.dropped });
    case "migration.children_not_migrated":
      return t("events:migration.children_not_migrated", { count: p.count });
    case "session.orphan_tool_result":
      return t("events:session.orphan_tool_result", { call_id: p.call_id ?? "" });
    case "session.unpaired_tool_use":
      return t("events:session.unpaired_tool_use", { tool_name: p.tool_name ?? "" });
    case "session.subagent_unlinked":
      return t("events:session.subagent_unlinked", { child_id: p.child_id ?? "" });
    case "session.child_foreign_ignored":
      return t("events:session.child_foreign_ignored", { child_id: p.child_id ?? "" });
    case "session.child_parent_conflict":
      return t("events:session.child_parent_conflict", { child_id: p.child_id ?? "" });
    case "codex.thread_unregistered":
      return t("events:codex.thread_unregistered", { session_id: p.session_id ?? "" });
    case "codex.thread_edge_unregistered":
      return t("events:codex.thread_edge_unregistered", { parent: p.parent ?? "", child: p.child ?? "" });
    case "edit.reply_replaced":
      return t("events:edit.reply_replaced", { turn: p.turn, items: p.items });
    case "edit.turn_deleted":
      return p.pruned_children
        ? t("events:edit.turn_deleted_with_children", { turn: p.turn, pruned_children: p.pruned_children })
        : t("events:edit.turn_deleted", { turn: p.turn });
    case "edit.message_rewritten":
      return t("events:edit.message_rewritten");
    default:
      return t("events:unknown_event_code", { code });
  }
}

export const renderEvents = list => (list || []).map(renderEvent);

const probeTextFor = (code, params) => {
  const p = params || {};
  if (code === "probe.process_failed") {
    if (p.exit_code != null) {
      return t("events:probe.process_failed_with_code", { exit_code: p.exit_code });
    }
    return t("events:probe.process_failed", { exit_code: "" });
  }
  const key = `events:probe.${code}`;
  const v = t(key, { ...p, defaultValue: null });
  return v != null ? v : code;
};

export const probeFailed = p =>
  !!p && (p.status === "failed" || p.ok === false);

export function probeText(p) {
  if (!p) return "";
  if (p.detail != null) return p.detail;
  const parts = [];
  if (p.isolation) {
    const kind = t(`events:isolation.${p.isolation.kind}`, { defaultValue: p.isolation.kind });
    parts.push(t("events:probe.isolation_cleanup", { kind, id: p.isolation.id ?? "" }));
  }
  if (p.code) parts.push(probeTextFor(p.code, p.params || {}));
  const d = p.diagnostic || {};
  if (d.stdout) parts.push(d.stdout);
  if (d.stderr) parts.push(d.stderr);
  if (d.truncated) parts.push(t("events:probe.truncated_suffix"));
  return parts.filter(Boolean).join("\n");
}

export function renderSnapshotReason(snapshot) {
  const code = snapshot?.reason_code;
  if (code) {
    const v = t(`events:snapshot.${code}`, { defaultValue: null });
    if (v != null) return v;
  }
  return snapshot?.legacy_reason || snapshot?.reason || t("events:snapshot.default_reason");
}
