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
