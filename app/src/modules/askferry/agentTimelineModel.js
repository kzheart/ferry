const READ_TOOLS = new Set(["session_search", "session_read", "usage"]);

export function mergeReadTools(run) {
  const rows = [];
  for (const item of run) {
    const mergeable = READ_TOOLS.has(item.name) && item.status !== "running";
    const previous = rows[rows.length - 1];
    if (mergeable && previous?._merge && previous.name === item.name) {
      previous.merged.push(item);
      previous.endedAt = item.endedAt;
    } else if (mergeable) {
      rows.push({ ...item, _merge: true, merged: [item] });
    } else {
      rows.push(item);
    }
  }
  return rows.map(row => (row._merge && row.merged.length === 1)
    ? (({ _merge, merged, ...item }) => item)(row)
    : row);
}

// 运行中但末尾条目没有自带活动指示(流式 spinner / 工具行 spinner)时,
// 消息流末尾需要补一个「思考中」占位,否则等首个 token 或工具间隙界面完全静止。
export function isAwaitingReply(status, items) {
  if (status !== "running") return false;
  const last = items[items.length - 1];
  if (!last) return true;
  if (last.kind === "user") return true;
  if (last.kind === "assistant") return !last.streaming;
  if (last.kind === "tool") return last.status !== "running";
  return false; // approval/status 各自有状态展示
}

export function groupAgentTimeline(items) {
  const grouped = [];
  let index = 0;
  while (index < items.length) {
    if (items[index].kind !== "tool") {
      grouped.push(items[index]);
      index += 1;
      continue;
    }
    const run = [];
    while (index < items.length && items[index].kind === "tool") {
      run.push(items[index]);
      index += 1;
    }
    grouped.push({ kind: "trace", rows: mergeReadTools(run) });
  }
  return grouped;
}
