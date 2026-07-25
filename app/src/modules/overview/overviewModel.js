// 总览页聚合:纯函数,从会话/迁移/快照 + 定价表算出各图表所需数据。
// 聚合逻辑不依赖 React；时间相关入参 now 便于测试。
import { repoOf } from "../browser/public.js";

const DAY = 86400e3;
export const TOKEN_KEYS = ["input", "output", "cache_read", "cache_write"];

export const emptyTokens = () => ({ input: 0, output: 0, cache_read: 0, cache_write: 0 });
export const addTokens = (a, b) => { TOKEN_KEYS.forEach(k => { a[k] += (b?.[k] || 0); }); return a; };
export const sumTokens = t => TOKEN_KEYS.reduce((s, k) => s + (t?.[k] || 0), 0);
const startedAt = s => s.created || s.updated || 0;

// ---------- 成本:模型名模糊匹配 models.dev 单价 ----------
const normModel = m => String(m || "").split("/").pop().toLowerCase();
const isBoundary = c => !c || /[-_.:]/.test(c);

export function buildPriceIndex(prices) {
  const idx = {};
  for (const key in prices) {
    const n = normModel(key);
    if (!(n in idx)) idx[n] = prices[key];
  }
  return idx;
}

// 归一后精确命中优先;否则取"互为前缀且边界对齐、长度最接近"的键。
export function matchPrice(model, prices, idx) {
  if (!model || !prices) return null;
  if (prices[model]) return prices[model];
  const n = normModel(model);
  idx = idx || buildPriceIndex(prices);
  if (idx[n]) return idx[n];
  let best = null, bestDiff = Infinity;
  for (const k in idx) {
    const short = n.length < k.length ? n : k;
    const long = n.length < k.length ? k : n;
    if (long.startsWith(short) && isBoundary(long[short.length])) {
      const diff = long.length - short.length;
      if (diff < bestDiff) { best = idx[k]; bestDiff = diff; }
    }
  }
  return best;
}

export function costOf(tokens, price) {
  if (!price) return 0;
  return (tokens.input * (price.input || 0) + tokens.output * (price.output || 0)
    + tokens.cache_read * (price.cache_read || 0)
    + tokens.cache_write * (price.cache_write || 0)) / 1e6;
}

// ---------- 主体聚合 ----------
export function computeOverview({ sessions = [], history = [], snaps = [],
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
  const costTotal = scoped.reduce((sum, s) =>
    sum + costOf(s.tokens || emptyTokens(), matchPrice(s.model, prices, idx)), 0);
  const prevTokens = prev.reduce((n, s) => n + sumTokens(s.tokens), 0);
  const prevCost = prev.reduce((sum, s) =>
    sum + costOf(s.tokens || emptyTokens(), matchPrice(s.model, prices, idx)), 0);
  const { streak, longest } = streaks(sessions, now);

  const composition = TOKEN_KEYS
    .map(key => ({ key, value: tokTotals[key], pct: total ? tokTotals[key] / total * 100 : 0 }))
    .sort((a, b) => b.value - a.value);

  // 成本表:按模型归并(可计价的按归一名合并 vip/、feature/ 等前缀重复)
  const byModel = new Map();
  scoped.forEach(s => {
    const tk = s.tokens; if (!tk) return;
    const price = matchPrice(s.model, prices, idx);
    const key = price ? normModel(s.model) : (s.model || "");
    const row = byModel.get(key) || { model: key, tokens: emptyTokens(), cost: 0, priced: !!price };
    addTokens(row.tokens, tk); row.cost += costOf(tk, price); row.priced = row.priced || !!price;
    byModel.set(key, row);
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

  const insights = buildInsights({ sessions, scoped, history, snaps, repos, prices,
    idx, now, nightShare: nightShare(), peakHour, clock, streak });

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
    insights,
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
    cost[i] += costOf(s.tokens || emptyTokens(), matchPrice(s.model, prices, idx));
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
    if (!s.model || !s.tokens) return;
    const mk = monthKey(startedAt(s));
    const col = keys.indexOf(mk);
    if (col < 0) return;
    const tk = sumTokens(s.tokens);
    perMonth[col].set(s.model, (perMonth[col].get(s.model) || 0) + tk);
    totalByModel.set(s.model, (totalByModel.get(s.model) || 0) + tk);
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

function daysSince(ts, now) { return Math.floor((now - ts) / DAY); }

// 洞察池:能从现有数据可靠触发的条目;组件按日轮换展示,不写"轮换"文案。
function buildInsights({ sessions, scoped, history, snaps, repos, prices, idx, now,
  nightShare, peakHour, clock, streak }) {
  const pool = [];

  // 闲置项目:最久未动且有一定体量的仓库
  const repoLast = new Map();
  sessions.forEach(s => {
    const r = repoOf(s.dir); if (!r) return;
    repoLast.set(r, Math.max(repoLast.get(r) || 0, s.updated || 0));
  });
  const repoCount = new Map();
  sessions.forEach(s => { const r = repoOf(s.dir); if (r) repoCount.set(r, (repoCount.get(r) || 0) + 1); });
  let idleRepo = null;
  repoLast.forEach((ts, name) => {
    const days = daysSince(ts, now);
    if ((repoCount.get(name) || 0) >= 5 && days >= 14 &&
        (!idleRepo || days > idleRepo.days)) idleRepo = { name, days };
  });
  if (idleRepo) pool.push({ kind: "idle", priority: 3, key: "idle",
    params: { repo: idleRepo.name, days: idleRepo.days } });

  // 来回迁移:同一 source_id 在两个工具间往返 ≥3 次
  const backForth = new Map();
  history.forEach(h => {
    const id = h.source_id || h.session_id || h.title; if (!id) return;
    backForth.set(id, (backForth.get(id) || 0) + 1);
  });
  let pingpong = null;
  backForth.forEach((n, id) => { if (n >= 3 && (!pingpong || n > pingpong.n)) {
    const row = history.find(h => (h.source_id || h.session_id || h.title) === id);
    pingpong = { n, title: row?.title || id };
  } });
  if (pingpong) pool.push({ kind: "pingpong", priority: 4, key: "pingpong",
    params: { title: pingpong.title, n: pingpong.n } });

  // 成本环比:近 7 天开销显著高于前 7 天的仓库
  const spike = costSpike(sessions, prices, idx, now);
  if (spike) pool.push({ kind: "cost", priority: 5, key: "cost", featured: true,
    params: spike });

  if (nightShare >= 0.4) pool.push({ kind: "night", priority: 2, key: "night",
    params: { pct: Math.round(nightShare * 100), hour: peakHour } });

  // 周末静默:最近一个完整周末(上周六日)无会话
  const wk = weekendSilent(sessions, now);
  if (wk) pool.push({ kind: "weekend", priority: 2, key: "weekend", params: {} });

  if (streak >= 7) pool.push({ kind: "streak", priority: 1, key: "streak",
    params: { days: streak } });

  // 快照占用:体量大的陈旧快照
  const snapBig = snapSpace(snaps, now);
  if (snapBig) pool.push({ kind: "snapshot", priority: 1, key: "snapshot", params: snapBig });

  // 兜底统计卡:洞察不足时补位,保证展示区不留空(与闲置卡同仓库时跳过,避免重复)
  if (repos[0] && repos[0].name !== idleRepo?.name) pool.push({ kind: "topRepo", priority: 0,
    key: "topRepo", params: { repo: repos[0].name, n: repos[0].total } });
  const tokByModel = new Map();
  let scopedTok = 0;
  scoped.forEach(s => {
    if (!s.model) return;
    const tk = sumTokens(s.tokens);
    tokByModel.set(s.model, (tokByModel.get(s.model) || 0) + tk);
    scopedTok += tk;
  });
  const topModel = [...tokByModel.entries()].sort((a, b) => b[1] - a[1])[0];
  if (topModel && scopedTok > 0) pool.push({ kind: "topModel", priority: 0, key: "topModel",
    params: { model: topModel[0], pct: Math.round(topModel[1] / scopedTok * 100) } });
  const activeDays = new Set();
  scoped.forEach(s => {
    const t = s.updated || 0; if (!t) return;
    const d = new Date(t); activeDays.add(new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime());
  });
  if (activeDays.size) pool.push({ kind: "avgDaily", priority: 0, key: "avgDaily",
    params: { n: (scoped.length / activeDays.size).toFixed(1) } });

  return pool.sort((a, b) => b.priority - a.priority);
}

function costSpike(sessions, prices, idx, now) {
  const wk = 7 * DAY;
  const recent = new Map(), before = new Map();
  sessions.forEach(s => {
    if (!s.tokens) return;
    const t = s.updated || 0; const r = repoOf(s.dir); if (!r) return;
    const c = costOf(s.tokens, matchPrice(s.model, prices, idx));
    if (t >= now - wk) recent.set(r, (recent.get(r) || 0) + c);
    else if (t >= now - 2 * wk) before.set(r, (before.get(r) || 0) + c);
  });
  let best = null;
  recent.forEach((cost, repo) => {
    const prevCost = before.get(repo) || 0;
    // 需上一周有开销才谈"涨",避免 prev=0 时的 +100% 误导
    if (cost >= 20 && prevCost > 0 && cost > prevCost * 1.4 && (!best || cost > best.cost)) {
      best = { repo, cost: Math.round(cost), pct: Math.round((cost / prevCost - 1) * 100),
        prev: Math.round(prevCost) };
    }
  });
  if (best) best.weeks = weeklyCost(sessions, best.repo, prices, idx, now, 5);
  return best;
}

// 指定仓库近 n 周的周开销序列(用于洞察趋势小图)
function weeklyCost(sessions, repo, prices, idx, now, n) {
  const wk = 7 * DAY;
  const series = new Array(n).fill(0);
  sessions.forEach(s => {
    if (!s.tokens || repoOf(s.dir) !== repo) return;
    const back = Math.floor((now - (s.updated || 0)) / wk);
    const i = n - 1 - back;
    if (i < 0 || i >= n) return;
    series[i] += costOf(s.tokens, matchPrice(s.model, prices, idx));
  });
  return series.map(v => Math.round(v));
}

function weekendSilent(sessions, now) {
  const active = new Set();
  sessions.forEach(s => {
    const t = s.updated || 0; if (!t) return;
    const d = new Date(t); active.add(new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime());
  });
  const d0 = new Date(now); const today = new Date(d0.getFullYear(), d0.getMonth(), d0.getDate()).getTime();
  const dow = new Date(today).getDay(); // 0=Sun
  const lastSun = today - (dow === 0 ? 7 : dow) * DAY;
  const lastSat = lastSun - DAY;
  return !active.has(lastSat) && !active.has(lastSun);
}

function snapSpace(snaps, now) {
  let best = null;
  snaps.forEach(s => {
    const size = s.size || 0; const days = daysSince(s.time || 0, now);
    if (size >= 500 * 1024 * 1024 && days >= 14 && (!best || size > best.bytes)) {
      best = { title: s.title || s.session || "", bytes: size, days,
        gb: (size / 1073741824).toFixed(1) };
    }
  });
  return best;
}
