// 总览页聚合:纯函数,从会话/迁移/快照 + 定价表算出各图表所需数据。
// 聚合逻辑不依赖 React；时间相关入参 now 便于测试。
import { repoOf } from "../browser/public.js";

const DAY = 86400e3;
const TOKEN_KEYS = ["input", "output", "cache_read", "cache_write"];

export const emptyTokens = () => ({ input: 0, output: 0, cache_read: 0, cache_write: 0 });
export const addTokens = (a, b) => { TOKEN_KEYS.forEach(k => { a[k] += (b?.[k] || 0); }); return a; };
export const sumTokens = t => TOKEN_KEYS.reduce((s, k) => s + (t?.[k] || 0), 0);
const startedAt = s => s.created || s.updated || 0;

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

// ---------- 主体聚合 ----------
export function computeOverview({ sessions = [], history = [],
  prices = {}, scope = "30", tool = "all", now = Date.now() } = {}) {
  if (tool && tool !== "all") {
    sessions = sessions.filter(s => s.tool === tool);
    history = history.filter(h => h.src === tool || h.dst === tool);
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

  // 迁移流向(src→dst 计数)
  const flowMap = new Map();
  history.forEach(h => {
    if (!h.src || !h.dst) return;
    const key = h.src + "\0" + h.dst;
    flowMap.set(key, (flowMap.get(key) || 0) + 1);
  });
  const flows = [...flowMap.entries()]
    .map(([k, count]) => { const [src, dst] = k.split("\0"); return { src, dst, count }; })
    .sort((a, b) => b.count - a.count).slice(0, 5);

  // 主力模型变迁(近 6 个自然月,按当月 token 排名)
  const bump = buildBump(sessions, now);

  const trends = buildTrends(sessions, prices, idx, now, 14);

  return {
    scope,
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
    flows,
    bump,
    trends,
    empty: !sessions.length,
    hasUsage: total > 0,
  };
}

// 近 n 天每日趋势(会话数 / token / 成本),供 KPI 迷你图
function buildTrends(sessions, prices, idx, now, n) {
  const d0 = new Date(now); const today = new Date(d0.getFullYear(), d0.getMonth(), d0.getDate()).getTime();
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
    const d = new Date(t); days.add(new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime());
  });
  if (!days.size) return { streak: 0, longest: 0 };
  const sorted = [...days].sort((a, b) => a - b);
  let longest = 1, run = 1;
  for (let i = 1; i < sorted.length; i++) {
    run = sorted[i] - sorted[i - 1] === DAY ? run + 1 : 1;
    longest = Math.max(longest, run);
  }
  // 当前连续:从今天或昨天往回数
  const d0 = new Date(now); const today = new Date(d0.getFullYear(), d0.getMonth(), d0.getDate()).getTime();
  let cur = 0, cursor = days.has(today) ? today : today - DAY;
  while (days.has(cursor)) { cur++; cursor -= DAY; }
  return { streak: cur, longest };
}

function buildHeatmap(sessions, now, weeks) {
  const perDay = new Map();
  sessions.forEach(s => {
    const t = s.updated || 0; if (!t) return;
    const d = new Date(t); const key = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    perDay.set(key, (perDay.get(key) || 0) + 1);
  });
  const d0 = new Date(now); const today = new Date(d0.getFullYear(), d0.getMonth(), d0.getDate()).getTime();
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

function monthKey(ts) { const d = new Date(ts); return d.getFullYear() * 12 + d.getMonth(); }

function buildBump(sessions, now) {
  const monthsBack = 6;
  const cur = monthKey(now);
  const keys = [];
  for (let i = monthsBack - 1; i >= 0; i--) keys.push(cur - i);
  const perMonth = keys.map(() => new Map());
  const totalByModel = new Map();
  sessions.forEach(s => {
    const mk = monthKey(startedAt(s));
    const col = keys.indexOf(mk);
    if (col < 0) return;
    modelUsageOf(s).forEach(({ model, tokens }) => {
      const tk = sumTokens(tokens);
      perMonth[col].set(model, (perMonth[col].get(model) || 0) + tk);
      totalByModel.set(model, (totalByModel.get(model) || 0) + tk);
    });
  });
  const top = [...totalByModel.entries()].sort((a, b) => b[1] - a[1]).slice(0, 4).map(e => e[0]);
  if (top.length < 2) return null;
  const monthLabels = keys.map(k => k % 12);   // 0-11 月份索引,由视图层按语言本地化
  const models = top.map(name => {
    const rank = keys.map((_, col) => {
      const ranked = top.map(m => [m, perMonth[col].get(m) || 0]).sort((a, b) => b[1] - a[1]);
      return ranked.findIndex(e => e[0] === name) + 1;
    });
    return { name, rank };
  });
  const lastCol = keys.length - 1;
  let leadName = null, leadRank = Infinity;
  models.forEach(m => { if (m.rank[lastCol] < leadRank) { leadRank = m.rank[lastCol]; leadName = m.name; } });
  models.forEach(m => { m.lead = m.name === leadName; });
  return { months: monthLabels, models, ranks: top.length };
}
