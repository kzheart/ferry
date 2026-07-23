export const BUCKETS = ["today", "yesterday", "last7", "last30", "earlier"];

const pad2 = n => String(n).padStart(2, "0");

// 相对时间格式化。t 为 i18n t 函数,由调用方注入(domain 层不依赖 React/i18next)。
// 不传 t 时回退到 key 字符串,保证 domain 纯函数可独立测试。
export function fmtTime(ms, t) {
  if (!ms) return t ? t("common:time.dash") : "—";
  const d = Date.now() - ms;
  if (d < 60e3) return t ? t("common:time.justNow") : "justNow";
  if (d < 3600e3) return t ? t("common:time.minutesAgo", { n: Math.floor(d / 60e3) }) : `${Math.floor(d / 60e3)}min`;
  if (d < 86400e3) return t ? t("common:time.hoursAgo", { n: Math.floor(d / 3600e3) }) : `${Math.floor(d / 3600e3)}hr`;
  if (d < 172800e3) {
    const tm = new Date(ms);
    const time = `${pad2(tm.getHours())}:${pad2(tm.getMinutes())}`;
    return t ? t("common:time.yesterdayAt", { time }) : `yesterday ${time}`;
  }
  if (d < 7 * 86400e3) {
    const n = Math.floor(d / 86400e3);
    return t ? t("common:time.daysAgo", { count: n }) : `${n}d`;
  }
  if (d < 30 * 86400e3) {
    const n = Math.floor(d / 7 / 86400e3);
    return t ? t("common:time.weeksAgo", { count: n }) : `${n}w`;
  }
  const tm = new Date(ms);
  return t ? t("common:time.monthDay", { date: tm })
    : `${tm.getMonth() + 1}/${tm.getDate()}`;
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

import { toolReferenceKind } from "../../api/contract/tools.js";

export const sessionRef = session =>
  toolReferenceKind(session.tool) === "id"
    ? session.id
    : (session.path || session.id);

export const operationRef = session => session.ref;

export function toRounds(messages, replyTurns) {
  if (replyTurns?.length) {
    return replyTurns.map(turn => {
      const userBlocks = turn.user?.blocks || [];
      const user = userBlocks.filter(block => block.kind === "text")
        .map(block => block.text).join("\n");
      const images = userBlocks.filter(block => block.kind === "image")
        .map(block => block.image).filter(Boolean);
      const seq = (turn.assistant_reply?.items || []).map(item => item.kind === "tool"
        ? { kind: "tool", tool: { ...item, size: item.output?.length || 0 } }
        : { kind: "text", text: item.text });
      const ai = seq.filter(item => item.kind === "text").map(item => item.text);
      const tools = seq.filter(item => item.kind === "tool").map(item => item.tool);
      let last = -1;
      seq.forEach((item, index) => { if (item.kind === "text") last = index; });
      return { n: turn.turn, user, images, locator: turn.turn_locator, index: turn.user?.index,
        ai, tools, seq, final: last >= 0 ? seq[last].text : "",
        steps: seq.filter((_, index) => index !== last), assistantReply: turn };
    });
  }
  const rounds = [];
  let current = null;
  for (const message of messages || []) {
    const texts = message.blocks.filter(block => block.kind === "text" && block.text.trim());
    const images = message.blocks.filter(block => block.kind === "image")
      .map(block => block.image).filter(Boolean);
    if (message.role === "user" && (texts.length || images.length)) {
      current = { n: rounds.length + 1, user: texts.map(text => text.text).join("\n"),
        images, locator: message.locator || message.uuid, index: message.index, ai: [], tools: [], seq: [] };
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

export function toTimeline(rounds, compactions) {
  const pending = new Map();
  for (const compaction of compactions || []) {
    const afterTurn = Number.isInteger(compaction.after_turn)
      ? compaction.after_turn : 0;
    pending.set(afterTurn, [...(pending.get(afterTurn) || []), compaction]);
  }
  const timeline = (pending.get(0) || []).map(compaction => ({
    kind: "compaction", key: `compaction:${compaction.id}`, compaction,
  }));
  for (const round of rounds || []) {
    timeline.push({ kind: "round", key: `round:${round.n}`, round });
    for (const compaction of pending.get(round.n) || []) {
      timeline.push({
        kind: "compaction",
        key: `compaction:${compaction.id}`,
        compaction,
      });
    }
  }
  for (const [afterTurn, items] of pending) {
    if (afterTurn <= (rounds?.length || 0)) continue;
    for (const compaction of items) {
      timeline.push({
        kind: "compaction",
        key: `compaction:${compaction.id}`,
        compaction,
      });
    }
  }
  return timeline;
}
