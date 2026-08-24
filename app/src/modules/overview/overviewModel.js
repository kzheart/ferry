// 总览页聚合:纯函数,从会话/迁移/快照 + 定价表算出各图表所需数据。
// 聚合逻辑不依赖 React；时间相关入参 now 便于测试。
import { repoOf } from "../browser/public.js";

const DAY = 86400e3;
const TOKEN_KEYS = ["input", "output", "cache_read", "cache_write"];

export const emptyTokens = () => ({ input: 0, output: 0, cache_read: 0, cache_write: 0 });
export const addTokens = (a, b) => { TOKEN_KEYS.forEach(k => { a[k] += (b?.[k] || 0); }); return a; };
export const sumTokens = t => TOKEN_KEYS.reduce((s, k) => s + (t?.[k] || 0), 0);
const startedAt = s => s.created || s.updated || 0;
const dayStartOf = ts => { const d = new Date(ts); return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(); };
// 热力等级 0-4,热力图与今日的近 7 天点阵共用同一档位
export const heatLevel = (count, max) => {
  if (count <= 0) return 0;
  const r = count / max;
  return r > 0.75 ? 4 : r > 0.5 ? 3 : r > 0.25 ? 2 : 1;
};

// ---------- 成本:模型名安全匹配多来源单价 ----------
const normFullModel = m => String(m || "").trim().toLowerCase().replaceAll("_", "-");
const normModel = m => normFullModel(m).split("/").pop();
const unsafeFuzzyPart = m => new Set(["auto", "mini", "chat", "default", "latest", "model"]).has(m);

export function buildPriceIndex(prices) {
  const idx = {};
  for (const key in prices) {
    const n = normModel(key);
    if (!(n in idx)) idx[n] = prices[key];
  }
  return idx;
}

// 只接受完整 key 或裸 model-part 的精确命中。旧实现会把 `gpt-5-mini` 按前缀
// 计成 `gpt-5`，这不是可证明的同一 SKU，会产生比“未计价”更危险的错误金额。
export function matchPrice(model, prices, idx) {
  if (!model || !prices) return null;
  const full = normFullModel(model);
  if (prices[full]) return prices[full];
  const n = normModel(model);
  if (!n || unsafeFuzzyPart(n)) return null;
  idx = idx || buildPriceIndex(prices);
  if (idx[n]) return idx[n];
  return null;
}

export function costOf(tokens, price) {
  if (!price) return 0;
  return (tokens.input * (price.input || 0) + tokens.output * (price.output || 0)
    + tokens.cache_read * (price.cache_read || 0)
    + tokens.cache_write * (price.cache_write || 0)) / 1e6;
}

// Scanner 会在树根保留按模型分桶；缺失时兼容旧数据，回退到会话代表模型。
export function modelUsageOf(session) {
  const entries = Object.entries(session?.usage_by_model || {})
    .filter(([model, tokens]) => model && tokens && typeof tokens === "object");
  if (entries.length) return entries.map(([model, tokens]) => ({ model, tokens }));
  return session?.model && session?.tokens
    ? [{ model: session.model, tokens: session.tokens }]
    : [];
}

// ---------- 今日速览 ----------
// 不受 scope 影响(“今天”本身就是时间范围),但沿用 agent 筛选后的会话集。
// token/成本按会话 updated 所在日整会话归属,与 buildTrends 的日桶同口径:跨天续聊的
// 会话会整体计入今天。刻意不给“今日消息数”——会话 count 是累计值,那个口径会虚高。
export function computeToday({ sessions = [], prices = {}, idx, now = Date.now() } = {}) {
  idx = idx || buildPriceIndex(prices);
  const today = dayStartOf(now);
  const elapsed = now - today;                 // 昨日同时段的截止偏移
  const todays = sessions.filter(s => (s.updated || 0) >= today);

  const tokTotals = emptyTokens();
  const agentMap = new Map(), modelMap = new Map(), projectMap = new Map();
  let costTotal = 0;
  todays.forEach(s => {
    addTokens(tokTotals, s.tokens);
    agentMap.set(s.tool, (agentMap.get(s.tool) || 0) + sumTokens(s.tokens));
    const repo = repoOf(s.dir);
    if (repo) projectMap.set(repo, (projectMap.get(repo) || 0) + 1);
    modelUsageOf(s).forEach(({ model, tokens: tk }) => {
      const price = matchPrice(model, prices, idx);
      const cost = costOf(tk, price);
      costTotal += cost;
      const key = price ? normModel(model) : (model || "");
      if (!key) return;
      const row = modelMap.get(key) || { model: key, tokens: 0, cost: 0, priced: !!price };
      row.tokens += sumTokens(tk); row.cost += cost; row.priced = row.priced || !!price;
      modelMap.set(key, row);
    });
  });
  const total = sumTokens(tokTotals);
  const share = value => total ? value / total * 100 : 0;

  const byAgent = [...agentMap.entries()]
    .filter(([, tokens]) => tokens > 0)
    .map(([tool, tokens]) => ({ tool, tokens, pct: share(tokens) }))
    .sort((a, b) => b.tokens - a.tokens);
  const byModel = [...modelMap.values()]
    .filter(r => r.tokens > 0)
    .map(r => ({ model: r.model, tokens: r.tokens, pct: share(r.tokens) }))
    .sort((a, b) => b.tokens - a.tokens);
  const costByModel = [...modelMap.values()]
    .filter(r => r.priced && r.cost > 0)
    .map(r => ({ model: r.model, cost: r.cost }))
    .sort((a, b) => b.cost - a.cost);
  const byProject = [...projectMap.entries()]
    .map(([name, count]) => ({ name, sessions: count }))
    .sort((a, b) => b.sessions - a.sessions || a.name.localeCompare(b.name));

  const composition = TOKEN_KEYS.map(key => ({ key, value: tokTotals[key] }));

  // 昨日同时段:截到与此刻相同的钟点,否则整个上午看都是暴跌
  const yesterday = { sessions: 0, tokens: 0, cost: 0 };
  const from = today - DAY, to = from + elapsed;
  sessions.forEach(s => {
    const u = s.updated || 0;
    if (u < from || u >= to) return;
    yesterday.sessions++;
    yesterday.tokens += sumTokens(s.tokens);
    yesterday.cost += modelUsageOf(s).reduce((sum, usage) =>
      sum + costOf(usage.tokens, matchPrice(usage.model, prices, idx)), 0);
  });
  const current = { sessions: todays.length, tokens: total, cost: costTotal };
  // 昨日同时段为 0 时百分比无意义(除零),返回 null 由视图退化显示
  const changeOf = (cur, prev) => prev > 0 ? (cur - prev) / prev * 100 : null;

  // 近 7 天活跃点阵(最后一格是今天),色阶与热力图一致
  const perDay = new Map();
  sessions.forEach(s => {
    const t = s.updated || 0; if (!t) return;
    const key = dayStartOf(t);
    perDay.set(key, (perDay.get(key) || 0) + 1);
  });
  const week = Array.from({ length: 7 }, (_, i) => {
    const day = today - (6 - i) * DAY;
    return { day, count: perDay.get(day) || 0 };
  });
  const weekMax = Math.max(1, ...week.map(d => d.count));
  week.forEach(d => { d.level = heatLevel(d.count, weekMax); });

  const topModel = byModel[0]?.model || null;
  const topModelPct = byModel[0]?.pct || 0;
  return {
    day: today,
    asOf: now,
    sessions: todays.length,
    created: todays.filter(s => (s.created || 0) >= today).length,
    tokens: total,
    cost: costTotal,
    composition,
    byAgent, byModel, costByModel, byProject,
    topModel,
    topModelPct,
    compare: {
      current, yesterday,
      tokensPct: changeOf(current.tokens, yesterday.tokens),
      costPct: changeOf(current.cost, yesterday.cost),
    },
    week,
    empty: todays.length === 0,
  };
}

// ---------- 主体聚合 ----------
export function computeOverview({ sessions = [],
  prices = {}, scope = "30", tool = "all", now = Date.now() } = {}) {
  if (tool && tool !== "all") {
    sessions = sessions.filter(s => s.tool === tool);
  }
  const idx = buildPriceIndex(prices);
  const win = scope === "7" ? 7 * DAY : scope === "30" ? 30 * DAY : Infinity;
  const from = win === Infinity ? 0 : now - win;
  const prevFrom = win === Infinity ? 0 : from - win;
  const scoped = sessions.filter(s => (s.updated || 0) >= from);
  const prev = win === Infinity ? [] : sessions.filter(s => {
    const u = s.updated || 0; return u >= prevFrom && u < from;
  });

  const tokTotals = emptyTokens();
  scoped.forEach(s => addTokens(tokTotals, s.tokens));
  const total = sumTokens(tokTotals);
  const sessionCost = s => modelUsageOf(s).reduce((sum, usage) =>
    sum + costOf(usage.tokens, matchPrice(usage.model, prices, idx)), 0);
  const costTotal = scoped.reduce((sum, s) => sum + sessionCost(s), 0);
  const prevTokens = prev.reduce((n, s) => n + sumTokens(s.tokens), 0);
  const prevCost = prev.reduce((sum, s) => sum + sessionCost(s), 0);
  const { streak, longest } = streaks(sessions, now);

  const composition = TOKEN_KEYS
    .map(key => ({ key, value: tokTotals[key], pct: total ? tokTotals[key] / total * 100 : 0 }))
    .sort((a, b) => b.value - a.value);

  // 成本表:按模型归并(可计价的按归一名合并 vip/、feature/ 等前缀重复)
  const byModel = new Map();
  scoped.forEach(s => {
    modelUsageOf(s).forEach(({ model, tokens: tk }) => {
      const price = matchPrice(model, prices, idx);
      const key = price ? normModel(model) : (model || "");
      const row = byModel.get(key) || { model: key, tokens: emptyTokens(), cost: 0, priced: !!price };
      addTokens(row.tokens, tk); row.cost += costOf(tk, price); row.priced = row.priced || !!price;
      byModel.set(key, row);
    });
  });
  const priced = [...byModel.values()].filter(r => r.priced && r.model)
    .map(r => ({ ...r, total: sumTokens(r.tokens) }))
    .sort((a, b) => b.cost - a.cost);
  const unpricedRows = [...byModel.values()].filter(r => !r.priced || !r.model);
  const unpriced = {
    models: unpricedRows.filter(r => r.model).length,
    tokens: unpricedRows.reduce((n, r) => n + sumTokens(r.tokens), 0),
  };
  const costRows = priced.slice(0, 5);

  // 作息时钟(24 桶,按会话开始时刻)
  const clock = new Array(24).fill(0);
  scoped.forEach(s => { const h = new Date(startedAt(s)).getHours(); clock[h]++; });
  const peakHour = clock.reduce((best, v, h) => v > clock[best] ? h : best, 0);
  const nightShare = total => {
    const night = clock.reduce((n, v, h) => (h <= 4 || h >= 21) ? n + v : n, 0);
    const all = clock.reduce((n, v) => n + v, 0);
    return all ? night / all : 0;
  };

  // 热力图:近 52 周(按 updated 计天,GitHub 风格整年)
  const heatmap = buildHeatmap(sessions, now, 52);

  // 仓库排行(按会话数,按工具拆分)
  const repoMap = new Map();
  scoped.forEach(s => {
    const name = repoOf(s.dir); if (!name) return;
    const r = repoMap.get(name) || { name, total: 0, byTool: {} };
    r.total++; r.byTool[s.tool] = (r.byTool[s.tool] || 0) + 1;
    repoMap.set(name, r);
  });
  const repos = [...repoMap.values()].sort((a, b) => b.total - a.total).slice(0, 6);

  // 用量走势(7/30 天按日,全部按周)与 Agent 份额对比
  const daily = buildDaily(scoped, prices, idx, now, scope);
  const agentShare = buildAgentShare(scoped, prev, win !== Infinity);

  const trends = buildTrends(sessions, prices, idx, now, 14);

  return {
    scope,
    today: computeToday({ sessions, prices, idx, now }),
    kpis: {
      sessions: { value: scoped.length, delta: win === Infinity ? null : scoped.length - prev.length },
      tokens: { value: total, delta: win === Infinity ? null : total - prevTokens },
      cost: { value: costTotal, delta: win === Infinity ? null : costTotal - prevCost },
      streak: { value: streak, longest },
    },
    tokenTotals: { ...tokTotals, total },
    composition,
    costRows, unpriced, costTotal,
    clock, peakHour, nightShare: nightShare(),
    heatmap,
    repos,
    daily,
    agentShare,
    trends,
    empty: !sessions.length,
    hasUsage: total > 0,
  };
}

// 近 n 天每日趋势(会话数 / token / 成本),供 KPI 迷你图
function buildTrends(sessions, prices, idx, now, n) {
  const today = dayStartOf(now);
  const sess = new Array(n).fill(0), tok = new Array(n).fill(0), cost = new Array(n).fill(0);
  sessions.forEach(s => {
    const t = s.updated || 0; if (!t) return;
    const day = Math.floor((today - t) / DAY);
    const i = n - 1 - day;
    if (i < 0 || i >= n) return;
    sess[i]++;
    tok[i] += sumTokens(s.tokens);
    cost[i] += modelUsageOf(s).reduce((sum, usage) =>
      sum + costOf(usage.tokens, matchPrice(usage.model, prices, idx)), 0);
  });
  return { sessions: sess, tokens: tok, cost };
}

// 连续活跃天数(当前 + 历史最长),按有会话活动的自然日
function streaks(sessions, now) {
  const days = new Set();
  sessions.forEach(s => {
    const t = s.updated || 0; if (!t) return;
    days.add(dayStartOf(t));
  });
  if (!days.size) return { streak: 0, longest: 0 };
  const sorted = [...days].sort((a, b) => a - b);
  let longest = 1, run = 1;
  for (let i = 1; i < sorted.length; i++) {
    run = sorted[i] - sorted[i - 1] === DAY ? run + 1 : 1;
    longest = Math.max(longest, run);
  }
  // 当前连续:从今天或昨天往回数
  const today = dayStartOf(now);
  let cur = 0, cursor = days.has(today) ? today : today - DAY;
  while (days.has(cursor)) { cur++; cursor -= DAY; }
  return { streak: cur, longest };
}

function buildHeatmap(sessions, now, weeks) {
  const perDay = new Map();
  sessions.forEach(s => {
    const t = s.updated || 0; if (!t) return;
    const key = dayStartOf(t);
    perDay.set(key, (perDay.get(key) || 0) + 1);
  });
  const today = dayStartOf(now);
  // 让最后一列落在本周;网格 [week][dow] dow 0=周一
  const dow = (new Date(today).getDay() + 6) % 7;
  const end = today + (6 - dow) * DAY;
  const start = end - (weeks * 7 - 1) * DAY;
  const grid = [];
  let max = 0, total = 0;
  for (let w = 0; w < weeks; w++) {
    const col = [];
    for (let d = 0; d < 7; d++) {
      const day = start + (w * 7 + d) * DAY;
      const count = day > today ? -1 : (perDay.get(day) || 0); // 未来日用 -1 占位
      if (count > 0) { total += count; max = Math.max(max, count); }
      col.push(count);
    }
    grid.push(col);
  }
  return { grid, max, total, weeks, start };
}

// 用量走势:7/30 天为日桶,"全部"为周桶(周一起始,最多 52 周,更早的被截掉)。
// 会话按 updated 所在日整体归桶,与 trends/热力图同口径。
const MAX_WEEKS = 52;

function buildDaily(scoped, prices, idx, now, scope) {
  const today = dayStartOf(now);
  const unit = scope === "all" ? "week" : "day";
  let starts;
  let truncated = false;
  if (unit === "day") {
    const n = scope === "7" ? 7 : 30;
    starts = Array.from({ length: n }, (_, i) => today - (n - 1 - i) * DAY);
  } else {
    const dow = (new Date(today).getDay() + 6) % 7;   // 0=周一
    const thisWeek = today - dow * DAY;
    const earliest = scoped.reduce((min, s) => {
      const u = s.updated || 0; return u && u < min ? u : min;
    }, now);
    const earliestDay = dayStartOf(earliest);
    const earliestWeek = earliestDay - ((new Date(earliestDay).getDay() + 6) % 7) * DAY;
    let n = Math.round((thisWeek - earliestWeek) / (7 * DAY)) + 1;
    if (n > MAX_WEEKS) { n = MAX_WEEKS; truncated = true; }
    if (n < 1) n = 1;
    starts = Array.from({ length: n }, (_, i) => thisWeek - (n - 1 - i) * 7 * DAY);
  }
  const span = unit === "day" ? DAY : 7 * DAY;
  const buckets = starts.map(start => ({ start, sessions: 0, tokens: 0, cost: 0, byTool: {} }));
  const toolTotals = new Map();
  scoped.forEach(s => {
    const u = s.updated || 0; if (!u) return;
    const i = Math.floor((dayStartOf(u) - starts[0]) / span);
    if (i < 0 || i >= buckets.length) return;
    const b = buckets[i];
    const tk = sumTokens(s.tokens);
    const cost = modelUsageOf(s).reduce((sum, usage) =>
      sum + costOf(usage.tokens, matchPrice(usage.model, prices, idx)), 0);
    b.sessions++; b.tokens += tk; b.cost += cost;
    const row = b.byTool[s.tool] || (b.byTool[s.tool] = { tokens: 0, cost: 0 });
    row.tokens += tk; row.cost += cost;
    toolTotals.set(s.tool, (toolTotals.get(s.tool) || 0) + tk);
  });
  // 堆叠顺序固定:总量大的在底部,过滤后颜色不重排
  const tools = [...toolTotals.entries()].sort((a, b) => b[1] - a[1]).map(e => e[0]);
  return {
    unit, buckets, tools, truncated,
    maxTokens: Math.max(0, ...buckets.map(b => b.tokens)),
    maxCost: Math.max(0, ...buckets.map(b => b.cost)),
  };
}

// Agent 份额:当期各 agent token 占比,及相对上一同长周期的百分点变化(scope=全部时无环比)
function buildAgentShare(scoped, prev, hasPrev) {
  const share = list => {
    const map = new Map();
    list.forEach(s => map.set(s.tool, (map.get(s.tool) || 0) + sumTokens(s.tokens)));
    const total = [...map.values()].reduce((a, b) => a + b, 0);
    return { map, total };
  };
  const cur = share(scoped);
  const before = share(prev);
  const pctOf = ({ map, total }, tool) => total ? (map.get(tool) || 0) / total * 100 : 0;
  return [...cur.map.entries()]
    .filter(([, tokens]) => tokens > 0)
    .map(([tool, tokens]) => ({
      tool, tokens,
      pct: pctOf(cur, tool),
      delta: hasPrev && before.total ? pctOf(cur, tool) - pctOf(before, tool) : null,
    }))
    .sort((a, b) => b.tokens - a.tokens);
}

// 某天明细(热力图/走势图点击下钻):会话数、token、成本、分 agent、Top 模型
export function computeDayDetail({ sessions = [], prices = {}, idx, day } = {}) {
  idx = idx || buildPriceIndex(prices);
  const todays = sessions.filter(s => {
    const u = s.updated || 0; return u >= day && u < day + DAY;
  });
  let tokens = 0, cost = 0;
  const agentMap = new Map(), modelMap = new Map();
  todays.forEach(s => {
    const tk = sumTokens(s.tokens);
    tokens += tk;
    agentMap.set(s.tool, (agentMap.get(s.tool) || 0) + tk);
    modelUsageOf(s).forEach(({ model, tokens: mt }) => {
      const price = matchPrice(model, prices, idx);
      cost += costOf(mt, price);
      const key = price ? normModel(model) : (model || "");
      if (key) modelMap.set(key, (modelMap.get(key) || 0) + sumTokens(mt));
    });
  });
  const pct = v => tokens ? v / tokens * 100 : 0;
  return {
    day,
    sessions: todays.length,
    tokens, cost,
    byAgent: [...agentMap.entries()].filter(([, v]) => v > 0)
      .map(([tool, v]) => ({ tool, tokens: v, pct: pct(v) }))
      .sort((a, b) => b.tokens - a.tokens),
    topModels: [...modelMap.entries()]
      .map(([model, v]) => ({ model, tokens: v, pct: pct(v) }))
      .sort((a, b) => b.tokens - a.tokens).slice(0, 3),
  };
}
