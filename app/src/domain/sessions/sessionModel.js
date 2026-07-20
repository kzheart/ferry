export const BUCKETS = [
  ["today", "今天"], ["yesterday", "昨天"], ["last7", "最近 7 天"],
  ["last30", "最近 30 天"], ["earlier", "更早"],
];

export function fmtTime(ms) {
  if (!ms) return "—";
  const d = Date.now() - ms;
  if (d < 60e3) return "刚刚";
  if (d < 3600e3) return Math.floor(d / 60e3) + " 分钟前";
  if (d < 86400e3) return Math.floor(d / 3600e3) + " 小时前";
  if (d < 172800e3) {
    const t = new Date(ms);
    return `昨天 ${String(t.getHours()).padStart(2, "0")}:${String(t.getMinutes()).padStart(2, "0")}`;
  }
  if (d < 7 * 86400e3) return Math.floor(d / 86400e3) + " 天前";
  if (d < 30 * 86400e3) return Math.floor(d / 7 / 86400e3) + " 周前";
  const t = new Date(ms);
  return `${t.getMonth() + 1} 月 ${t.getDate()} 日`;
}

export function bucketOf(ms) {
  if (!ms) return "earlier";
  const now = new Date();
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (ms >= midnight) return "today";
  if (ms >= midnight - 86400e3) return "yesterday";
  if (ms >= midnight - 6 * 86400e3) return "last7";
  if (ms >= midnight - 29 * 86400e3) return "last30";
  return "earlier";
}

export function repoOf(dir) {
  if (!dir) return "";
  const parts = String(dir).split("/").filter(Boolean);
  return parts[parts.length - 1] || dir;
}

export const sessionRef = session =>
  session.tool === "opencode" ? session.id : (session.path || session.id);

export function toRounds(messages, authoredTurns) {
  if (authoredTurns?.length) {
    return authoredTurns.map(turn => {
      const userBlocks = turn.user?.blocks || [];
      const user = userBlocks.filter(block => block.kind === "text")
        .map(block => block.text).join("\n");
      const seq = (turn.assistant_reply?.items || []).map(item => item.kind === "tool"
        ? { kind: "tool", tool: { ...item, size: item.output?.length || 0 } }
        : { kind: "text", text: item.text });
      const ai = seq.filter(item => item.kind === "text").map(item => item.text);
      const tools = seq.filter(item => item.kind === "tool").map(item => item.tool);
      let last = -1;
      seq.forEach((item, index) => { if (item.kind === "text") last = index; });
      return { n: turn.turn, user, locator: turn.turn_locator, index: turn.user?.index,
        ai, tools, seq, final: last >= 0 ? seq[last].text : "",
        steps: seq.filter((_, index) => index !== last), authoring: turn };
    });
  }
  const rounds = [];
  let current = null;
  for (const message of messages || []) {
    const texts = message.blocks.filter(block => block.kind === "text" && block.text.trim());
    if (message.role === "user" && texts.length) {
      current = { n: rounds.length + 1, user: texts.map(text => text.text).join("\n"),
        locator: message.locator || message.uuid, index: message.index, ai: [], tools: [], seq: [] };
      rounds.push(current);
      continue;
    }
    if (!current) {
      current = { n: 1, user: "", locator: null, index: message.index, ai: [], tools: [], seq: [] };
      rounds.push(current);
    }
    message.blocks.forEach(block => {
      if (block.kind === "text" && message.role === "assistant" && block.text.trim()) {
        current.ai.push(block.text);
        current.seq.push({ kind: "text", text: block.text });
      }
      if (block.kind === "tool") {
        current.tools.push(block);
        current.seq.push({ kind: "tool", tool: block });
      }
    });
  }
  for (const round of rounds) {
    let last = -1;
    round.seq.forEach((step, index) => { if (step.kind === "text") last = index; });
    round.final = last >= 0 ? round.seq[last].text : "";
    round.steps = round.seq.filter((_, index) => index !== last);
  }
  return rounds;
}
