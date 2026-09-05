export const BUCKETS = ["today", "yesterday", "last7", "last30", "earlier"];

const pad2 = n => String(n).padStart(2, "0");

// 相对时间格式化。t 为 i18n t 函数，由调用方注入。
// 不传 t 时回退到 key 字符串，保证纯函数可独立测试。
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

// 按 `/` 或 `\` 拆路径段。Windows 会话 cwd 是 `D:\code\ferry`，只认斜杠会把整段
// 路径当成仓库名画进侧栏。
export function pathSegments(dir) {
  return String(dir).replace(/^(?:\\\\\?\\|\/\/\?\/)/, "")
    .replace(/\\/g, "/").split("/").filter(Boolean);
}

export function isWindowsProjectPath(dir) {
  const text = String(dir || "").replace(/^(?:\\\\\?\\|\/\/\?\/)/, "");
  return /^[A-Za-z]:[\\/]/.test(text) || /^(?:\\\\|\/\/)[^\\/]/.test(text);
}

// Agent 会把同一个 Windows cwd 写成 `C:\\work\\app`、`C:/work/app`，有时还
// 带 Win32 的 `\\?\\` 前缀。项目身份不能直接使用这些原始字符串，否则跨 Agent
// 的同一目录会被拆成多个文件夹。这里只做词法归一化，不访问文件系统：历史会话
// 的目录可能已经不存在，也不能让符号链接解析改变 macOS 上的项目身份。
export function normalizeProjectPath(dir) {
  if (!dir) return "";
  let text = String(dir).replace(/^(?:\\\\\?\\|\/\/\?\/)/, "");
  const windows = isWindowsProjectPath(text);
  if (windows) {
    const unc = /^(?:\\\\|\/\/)/.test(text);
    text = text.replace(/[\\/]+/g, "\\");
    if (unc) text = `\\\\${text.replace(/^\\+/, "")}`;
    if (/^[A-Za-z]:/.test(text)) text = `${text[0].toUpperCase()}${text.slice(1)}`;
    while (text.endsWith("\\") && !/^[A-Za-z]:\\$/.test(text)) text = text.slice(0, -1);
    return text;
  }
  text = text.replace(/\/{2,}/g, "/");
  while (text.length > 1 && text.endsWith("/")) text = text.slice(0, -1);
  return text;
}

// Windows 文件系统默认大小写不敏感；macOS 可能运行在大小写敏感卷上，因此只对
// 明确长得像 Windows 的路径折叠大小写。
export function projectPathKey(dir) {
  const normalized = normalizeProjectPath(dir);
  const windows = isWindowsProjectPath(normalized);
  return `${windows ? "win" : "posix"}:${windows ? normalized.toLowerCase() : normalized}`;
}

export function repoOf(dir) {
  if (!dir) return "";
  const parts = pathSegments(dir);
  return parts[parts.length - 1] || "";
}

// 会话引用只能由 Engine 签发；路径和各 Agent 原生 ID 不得进入 UI 调用链。
export const sessionRef = session => session.ref;

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

export function toTimeline(rounds, compactions, hasMore = false) {
  const pending = new Map();
  for (const compaction of compactions || []) {
    const afterTurn = Number.isInteger(compaction.after_turn)
      ? compaction.after_turn : 0;
    pending.set(afterTurn, [...(pending.get(afterTurn) || []), compaction]);
  }
  const timeline = [];
  // 同一位置的多次压缩合并为一个分组项，由 UI 折叠展示。
  const pushGroup = items => {
    if (!items?.length) return;
    timeline.push({
      kind: "compaction",
      key: `compaction:${items[0].id}`,
      compactions: items,
    });
  };
  pushGroup(pending.get(0));
  let lastTurn = 0;
  for (const round of rounds || []) {
    timeline.push({ kind: "round", key: `round:${round.n}`, round });
    if (round.n > lastTurn) lastTurn = round.n;
    pushGroup(pending.get(round.n));
  }
  // 尚未加载到对应轮次的压缩点先不渲染，等分页加载归位；
  // 全部加载完后才把超出范围的兜底追加到末尾。
  if (hasMore) return timeline;
  for (const [afterTurn, items] of pending) {
    if (afterTurn <= lastTurn) continue;
    pushGroup(items);
  }
  return timeline;
}
