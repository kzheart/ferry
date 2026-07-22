// 总览页:KPI + 使用习惯 + Token/成本 + 项目/迁移 + 洞察。
// 数据全部由 computeOverview 从真实扫描结果聚合;图表手写内联 SVG,随主题变量着色。
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOLS, TOOL_NAME } from "../../api/contract/tools.js";
import { ToolIcon, SortCaret, CheckIcon, RailGlyph, Spinner } from "../../components/ui/icons.jsx";
import { computeOverview } from "../../domain/sessions/overviewModel.js";

const TOOL_COLOR = { claude: "var(--t-claude)", codex: "var(--t-codex)", opencode: "var(--t-opencode)" };
const COMP_OPACITY = { cache_read: 0.92, input: 0.6, cache_write: 0.38, output: 0.2 };
const CHART = ["var(--c1)", "var(--c2)", "var(--c3)", "var(--c4)"];

// ---------- 格式化 ----------
const fmtInt = n => Math.round(n || 0).toLocaleString("en-US");
function fmtTokens(n) {
  n = n || 0;
  if (n >= 1e9) return (n / 1e9).toFixed(2) + " B";
  if (n >= 1e6) return (n / 1e6).toFixed(1) + " M";
  if (n >= 1e3) return (n / 1e3).toFixed(1) + " K";
  return String(Math.round(n));
}
const fmtCost = n => "$" + Math.round(n || 0).toLocaleString("en-US");

// 月份/星期名按语言本地化。2024-01-01 是周一,用它推出 dow 0=周一 的顺序。
const monthName = (index, locale) =>
  new Date(2024, index, 1).toLocaleDateString(locale, { month: "short" });
const weekdayNames = locale =>
  Array.from({ length: 7 }, (_, d) =>
    new Date(2024, 0, 1 + d).toLocaleDateString(locale, { weekday: "short" }));

// Catmull-Rom → 三次贝塞尔平滑
function smooth(pts) {
  if (pts.length < 2) return "";
  let d = `M${pts[0][0].toFixed(2)} ${pts[0][1].toFixed(2)}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[i === 0 ? 0 : i - 1], p1 = pts[i], p2 = pts[i + 1];
    const p3 = pts[i + 2 < pts.length ? i + 2 : i + 1];
    const c1x = p1[0] + (p2[0] - p0[0]) / 6, c1y = p1[1] + (p2[1] - p0[1]) / 6;
    const c2x = p2[0] - (p3[0] - p1[0]) / 6, c2y = p2[1] - (p3[1] - p1[1]) / 6;
    d += ` C${c1x.toFixed(2)} ${c1y.toFixed(2)},${c2x.toFixed(2)} ${c2y.toFixed(2)},${p2[0].toFixed(2)} ${p2[1].toFixed(2)}`;
  }
  return d;
}

// ---------- 通用壳 ----------
const card = { background: "var(--surface)", border: "1px solid var(--line)",
  borderRadius: 10, boxShadow: "var(--shadow)" };
const num = { fontVariantNumeric: "tabular-nums" };

function Card({ title, sub, extra, fill, children }) {
  return (
    <div style={fill ? { ...card, height: "100%", display: "flex", flexDirection: "column" } : card}>
      <div style={{ display: "flex", alignItems: "baseline", gap: 10, padding: "13px 15px 0" }}>
        <h2 style={{ margin: 0, fontSize: 12, fontWeight: 600, letterSpacing: ".02em", color: "var(--tx2)" }}>{title}</h2>
        {sub && <span style={{ fontSize: 11, color: "var(--tx4b)" }}>{sub}</span>}
        {extra && <><div style={{ flex: 1 }} />{extra}</>}
      </div>
      <div style={fill ? { padding: "13px 15px 15px", flex: 1, display: "flex", flexDirection: "column" }
        : { padding: "13px 15px 15px" }}>{children}</div>
    </div>
  );
}

function Section({ title, note }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 10, margin: "4px 0 -8px" }}>
      <h3 style={{ margin: 0, fontSize: 12, fontWeight: 600, color: "var(--tx2)", letterSpacing: ".01em" }}>{title}</h3>
      <div style={{ flex: 1, height: 1, background: "var(--line)" }} />
      {note && <span style={{ fontSize: 11, color: "var(--tx4b)" }}>{note}</span>}
    </div>
  );
}

// ---------- 图表 ----------
function Spark({ values, w = 72, h = 24 }) {
  const max = Math.max(1, ...values), pad = 2;
  const x = i => pad + (i / Math.max(1, values.length - 1)) * (w - pad * 2);
  const y = v => h - pad - (v / max) * (h - pad * 2);
  const pts = values.map((v, i) => [x(i), y(v)]);
  const d = smooth(pts);
  return (
    <svg viewBox={`0 0 ${w} ${h}`} width={w} height={h} style={{ display: "block" }} aria-hidden="true">
      <path d={`${d} L${x(values.length - 1)} ${h} L${x(0)} ${h} Z`} fill="var(--c1)" opacity=".1" />
      <path d={d} fill="none" stroke="var(--c1)" strokeWidth="1.3" strokeLinejoin="round" strokeLinecap="round" opacity=".75" />
      <circle cx={x(values.length - 1)} cy={y(values[values.length - 1] || 0)} r="2" fill="var(--c1)" />
    </svg>
  );
}

function Bump({ bump, locale, label }) {
  const W = 720, H = 200, L = 60, R = 110, T = 22, B = 26;
  const { months, models, ranks } = bump;
  const n = months.length;
  const x = i => L + (i / (n - 1)) * (W - L - R);
  const y = r => T + ((r - 1) / Math.max(1, ranks - 1)) * (H - T - B);
  const order = models.map((m, i) => i).sort((a, b) => (models[a].lead ? 1 : 0) - (models[b].lead ? 1 : 0));
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height="100%" preserveAspectRatio="xMidYMid meet"
      style={{ display: "block" }} role="img"
      aria-label={label}>
      {Array.from({ length: ranks }, (_, i) => i + 1).map(r => (
        <g key={r}>
          <line x1={L} x2={W - R} y1={y(r)} y2={y(r)} stroke="var(--grid)" strokeWidth="1" />
          <text x={L - 14} y={y(r) + 3.5} textAnchor="middle" fill="var(--tx5)" fontSize="10" fontFamily="var(--font-ui)">#{r}</text>
        </g>
      ))}
      {months.map((m, i) => (
        <text key={i} x={x(i)} y={H - 8} textAnchor="middle" fill="var(--tx5)" fontSize="10" fontFamily="var(--font-ui)">{monthName(m, locale)}</text>
      ))}
      {order.map(mi => {
        const m = models[mi];
        const color = CHART[mi % CHART.length];
        const pts = m.rank.map((r, i) => [x(i), y(r)]);
        return (
          <g key={m.name}>
            <path d={smooth(pts)} fill="none" stroke={color}
              strokeWidth={m.lead ? 2.4 : 1.6} strokeLinecap="round" strokeLinejoin="round" opacity={m.lead ? 1 : 0.45} />
            {pts.map((p, i) => (
              <circle key={i} cx={p[0]} cy={p[1]} r={m.lead ? (i === n - 1 ? 4 : 3) : 2.4}
                fill={m.lead ? color : "var(--surface)"} stroke={m.lead ? "var(--surface)" : color}
                strokeWidth={m.lead ? 1.4 : 1.2} opacity={m.lead ? 1 : 0.6} />
            ))}
            <text x={W - R + 10} y={y(m.rank[n - 1]) + 3.5} fill={color}
              fontSize="11" fontWeight={m.lead ? 600 : 400} fontFamily="var(--font-ui)"
              opacity={m.lead ? 1 : 0.75}>{m.name}</text>
          </g>
        );
      })}
    </svg>
  );
}

function Clock({ clock, peakHour, t }) {
  const CX = 130, CY = 112, R0 = 30, R1 = 94;
  const max = Math.max(1, ...clock);
  const step = (Math.PI * 2) / 24, gap = step * 0.14;
  const wedge = (i, val) => {
    const a0 = -Math.PI / 2 + i * step + gap / 2;
    const a1 = -Math.PI / 2 + (i + 1) * step - gap / 2;
    const r = R0 + (val / max) * (R1 - R0);
    const p = (ang, rad) => [(CX + Math.cos(ang) * rad).toFixed(2), (CY + Math.sin(ang) * rad).toFixed(2)];
    const A = p(a0, R0), Bp = p(a1, R0), C = p(a1, r), D = p(a0, r);
    return `M${A[0]} ${A[1]} A${R0} ${R0} 0 0 1 ${Bp[0]} ${Bp[1]} L${C[0]} ${C[1]} A${r} ${r} 0 0 0 ${D[0]} ${D[1]} Z`;
  };
  return (
    <>
    <svg viewBox="0 0 260 232" width="100%" height="232" role="img" aria-label={t("overview:clock.aria")}>
      {[0.5, 1].map(f => (
        <circle key={f} cx={CX} cy={CY} r={R0 + (R1 - R0) * f} fill="none" stroke="var(--grid)" strokeWidth="1" />
      ))}
      {clock.map((v, h) => {
        const late = h <= 4 || h >= 21;
        return <path key={h} d={wedge(h, v)} fill={late ? "var(--warn)" : "var(--c1)"} opacity={late ? 0.9 : 0.45} />;
      })}
      {[[0, "0"], [6, "6"], [12, "12"], [18, "18"]].map(([hh, lb]) => {
        const ang = -Math.PI / 2 + (hh + 0.5) * step, rr = R1 + 15;
        return <text key={hh} x={(CX + Math.cos(ang) * rr).toFixed(1)} y={(CY + Math.sin(ang) * rr + 3.5).toFixed(1)}
          textAnchor="middle" fill="var(--tx5)" fontSize="10" fontFamily="var(--font-ui)">{lb}</text>;
      })}
      <text x={CX} y={CY - 2} textAnchor="middle" fill="var(--tx1)" fontSize="17" fontWeight="600"
        fontFamily="var(--font-ui)" letterSpacing="-0.5">{String(peakHour).padStart(2, "0")}:00</text>
      <text x={CX} y={CY + 12} textAnchor="middle" fill="var(--tx4b)" fontSize="10" fontFamily="var(--font-ui)">{t("overview:clock.peak")}</text>
    </svg>
    <div style={{ display: "flex", justifyContent: "center", gap: 18, marginTop: 2 }}>
      {[["var(--warn)", 0.9, t("overview:clock.night")], ["var(--c1)", 0.45, t("overview:clock.day")]].map(([fill, op, lb], i) => (
        <span key={i} style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 11, color: "var(--tx5)", fontFamily: "var(--font-ui)" }}>
          <span style={{ width: 8, height: 8, borderRadius: 2, background: fill, opacity: op }} />
          {lb}
        </span>
      ))}
    </div>
    </>
  );
}

function Heatmap({ heatmap, locale, label }) {
  const { grid, max } = heatmap;
  const cell = 11, gap = 3, LX = 26, TY = 8;
  const level = c => {
    if (c <= 0) return 0;
    const r = c / max;
    return r > 0.75 ? 4 : r > 0.5 ? 3 : r > 0.25 ? 2 : 1;
  };
  const width = LX + grid.length * (cell + gap);
  // 隔行显示(周一/三/五/日),避免标签挤在一起
  const names = weekdayNames(locale);
  const dow = names.map((s, d) => (d % 2 === 0 ? s : ""));
  return (
    <svg viewBox={`0 0 ${width} ${TY + 7 * (cell + gap)}`} width="100%"
      style={{ display: "block", minWidth: width }}
      role="img" aria-label={label}>
      {dow.map((s, d) => s && (
        <text key={d} x={LX - 6} y={TY + d * (cell + gap) + 9} textAnchor="end" fill="var(--tx5)" fontSize="9" fontFamily="var(--font-ui)">{s}</text>
      ))}
      {grid.map((col, w) => col.map((c, d) => c === -1 ? null : (
        <rect key={`${w}-${d}`} x={LX + w * (cell + gap)} y={TY + d * (cell + gap)}
          width={cell} height={cell} rx="2.5" fill={`var(--heat-${level(c)})`}>
          <title>{c > 0 ? `${c}` : ""}</title>
        </rect>
      )))}
    </svg>
  );
}

function MiniBars({ weeks, label }) {
  const W = 150, H = 86, B = 16, T = 6;
  const max = Math.max(1, ...weeks), bw = W / weeks.length;
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="150" height="86" aria-label={label}>
      {weeks.map((v, i) => {
        const last = i === weeks.length - 1, hh = (v / max) * (H - T - B);
        return (
          <g key={i}>
            <rect x={i * bw + 4} y={H - B - hh} width={bw - 8} height={hh} rx="2"
              fill={last ? "var(--warn)" : "var(--accent)"} opacity={last ? 0.95 : 0.18} />
            {last && <text x={i * bw + bw / 2} y={H - B - hh - 5} textAnchor="middle" fill="var(--warn)"
              fontSize="10" fontWeight="600" fontFamily="var(--font-ui)">{fmtCost(v)}</text>}
          </g>
        );
      })}
      <line x1="2" x2={W - 2} y1={H - B} y2={H - B} stroke="var(--line2)" strokeWidth="1" />
      <text x={W / 2} y={H - 3} textAnchor="middle" fill="var(--tx5)" fontSize="9.5" fontFamily="var(--font-ui)">{label}</text>
    </svg>
  );
}

// Agent 筛选:产品图标下拉,清单来自引擎 tools RPC,新增 agent 自动出现
function ToolFilter({ tool, setTool, t }) {
  const [open, setOpen] = useState(false);
  const label = tool === "all" ? t("overview:filter.all") : (TOOL_NAME[tool] || tool);
  const pick = k => { setTool(k); setOpen(false); };
  const row = (k, icon, text) => (
    <div key={k} role="option" aria-selected={tool === k} onClick={() => pick(k)} className="hov-item"
      style={{ display: "flex", alignItems: "center", gap: 8, height: 32, padding: "0 8px",
        borderRadius: 6, fontSize: 12, color: "var(--tx2)", cursor: "default", whiteSpace: "nowrap" }}>
      {icon}
      <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>{text}</span>
      {tool === k && <span style={{ color: "var(--tx3)", display: "inline-flex" }}><CheckIcon size={12} /></span>}
    </div>
  );
  const allIcon = (
    <span style={{ width: 20, height: 20, borderRadius: 6, background: "var(--fill3)",
      border: "1px solid var(--line)", display: "inline-flex", alignItems: "center",
      justifyContent: "center", flex: "none" }}>
      <RailGlyph name="overview" size={11} color="var(--tx3b)" />
    </span>
  );
  return (
    <div style={{ position: "relative" }}>
      <button onClick={() => setOpen(o => !o)} aria-haspopup="listbox" aria-expanded={open}
        aria-label={t("overview:filter.label")}
        style={{ display: "inline-flex", alignItems: "center", gap: 7, height: 28, padding: "0 9px",
          border: "1px solid var(--line)", borderRadius: 6, background: "var(--surface)",
          font: "inherit", fontSize: 12, color: "var(--tx1)", cursor: "default" }}>
        {tool === "all" ? allIcon : <ToolIcon tool={tool} size={18} />}
        {label}
        <SortCaret />
      </button>
      {open && (
        <>
          <div onClick={() => setOpen(false)} style={{ position: "fixed", inset: 0, zIndex: 55 }} />
          <div role="listbox" style={{ position: "absolute", right: 0, top: "calc(100% + 6px)",
            zIndex: 56, minWidth: 176, maxHeight: 300, overflowY: "auto", padding: 6,
            background: "var(--bg)", borderRadius: 10,
            boxShadow: "var(--shadow-menu)",
             }}>
            {row("all", allIcon, t("overview:filter.all"))}
            <div style={{ height: 1, background: "var(--line3)", margin: "4px 6px" }} />
            {TOOLS.map(k => row(k, <ToolIcon tool={k} size={20} />, TOOL_NAME[k] || k))}
          </div>
        </>
      )}
    </div>
  );
}

// ---------- 洞察文案 ----------
function insightCopy(ins, t) {
  const p = ins.params;
  return {
    eyebrow: t(`overview:ins.${ins.kind}.eyebrow`),
    title: t(`overview:ins.${ins.kind}.title`, p),
    body: t(`overview:ins.${ins.kind}.body`, { ...p, cost: fmtCost(p.cost || 0), prev: fmtCost(p.prev || 0) }),
  };
}

// ---------- 主组件 ----------
export default function Overview({ sessions = [], historyRows = [],
  prices = {}, scanning = false }) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const [scope, setScope] = useState("30");
  const [tool, setTool] = useState("all");
  const data = useMemo(() => computeOverview({
    sessions, history: historyRows, prices, scope, tool,
  }), [sessions, historyRows, prices, scope, tool]);

  const delta = (kpi, fmt) => {
    if (kpi.delta == null) return null;
    const up = kpi.delta >= 0;
    return <span style={{ fontSize: 11, fontWeight: 500, color: up ? "var(--ok)" : "var(--err)", ...num }}>
      {up ? "+" : "−"}{fmt(Math.abs(kpi.delta))} {t("overview:kpi.thisPeriod")}
    </span>;
  };

  // 洞察按日轮换选取(无感,不写轮换文案):成本预警占主推,其余轮换成小卡
  const featured = data.insights.find(i => i.featured) || null;
  const rest = data.insights.filter(i => i !== featured);
  const offset = rest.length ? Math.floor(Date.now() / 864e5) % rest.length : 0;
  const smalls = rest.length
    ? rest.slice(offset).concat(rest.slice(0, offset)).slice(0, featured ? 3 : 4)
    : [];

  const segBtn = (label, active, onClick, dot) => (
    <button key={label} onClick={onClick} aria-pressed={active}
      style={{ border: "none", background: active ? "var(--surface)" : "transparent",
        font: "inherit", fontSize: 12, color: active ? "var(--tx1)" : "var(--tx3)",
        fontWeight: active ? 500 : 400, padding: "3px 10px", borderRadius: 5, cursor: "default",
        display: "inline-flex", alignItems: "center", gap: 6,
        boxShadow: active ? "0 1px 1px rgba(0,0,0,.05)" : "none" }}>
      {dot && <i style={{ width: 7, height: 7, borderRadius: 2, background: dot, opacity: active ? 1 : 0.55 }} />}
      {label}
    </button>
  );
  const scopeBtn = key => segBtn(t(`overview:scope.${key}`), scope === key, () => setScope(key));

  const kpiCard = (label, value, unit, footLeft, sparkVals) => (
    <div style={{ ...card, padding: "13px 14px 10px" }}>
      <span style={{ fontSize: 11, color: "var(--tx4b)" }}>{label}</span>
      <div style={{ fontSize: 26, fontWeight: 600, letterSpacing: "-0.03em", marginTop: 3,
        display: "flex", alignItems: "baseline", gap: 4, ...num }}>
        {value}{unit && <span style={{ fontSize: 13, fontWeight: 500, color: "var(--tx4)", letterSpacing: 0 }}>{unit}</span>}
      </div>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", gap: 8, marginTop: 4 }}>
        {footLeft}
        <Spark values={sparkVals} />
      </div>
    </div>
  );

  return (
    <div className="fscroll" style={{ flex: 1, minWidth: 0, overflowY: "auto",
      background: "var(--bg)" }}>
      <div style={{ maxWidth: 1180, margin: "0 auto", padding: "22px 24px 60px",
        display: "flex", flexDirection: "column", gap: 22 }}>

        {/* 顶部:标题 + 时间范围 */}
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <h1 style={{ margin: 0, fontSize: 15, fontWeight: 600, letterSpacing: "-0.01em", color: "var(--tx1)" }}>{t("overview:title")}</h1>
          <div style={{ flex: 1 }} />
          <ToolFilter tool={tool} setTool={setTool} t={t} />
          <div role="group" aria-label={t("overview:scope.label")}
            style={{ display: "flex", background: "var(--track)", borderRadius: 6, padding: 2, gap: 2 }}>
            {scopeBtn("7")}{scopeBtn("30")}{scopeBtn("all")}
          </div>
        </div>

        {data.empty ? (
          <div style={{ ...card, padding: "48px 20px", color: "var(--tx5)", fontSize: 13,
            display: "flex", alignItems: "center", justifyContent: "center", gap: 8 }}>
            {/* 首次扫描尚未完成时是加载中,不是"没有会话",避免误导去手动重扫 */}
            {scanning ? <><Spinner /> {t("overview:scanning")}</> : t("overview:emptyState")}
          </div>
        ) : (
          <>
            {/* KPI */}
            <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))", gap: 12 }}>
              {kpiCard(t("overview:kpi.sessions"), fmtInt(data.kpis.sessions.value), null,
                delta(data.kpis.sessions, fmtInt), data.trends.sessions)}
              {kpiCard(t("overview:kpi.tokens"), fmtTokens(data.kpis.tokens.value).split(" ")[0],
                fmtTokens(data.kpis.tokens.value).split(" ")[1] || "",
                delta(data.kpis.tokens, v => fmtTokens(v)), data.trends.tokens)}
              {kpiCard(t("overview:kpi.cost"), fmtCost(data.kpis.cost.value), null,
                delta(data.kpis.cost, fmtCost), data.trends.cost)}
              {kpiCard(t("overview:kpi.streak"), data.kpis.streak.value, t("overview:kpi.days"),
                <span style={{ fontSize: 11, color: "var(--tx4b)", ...num }}>{t("overview:kpi.longest", { n: data.kpis.streak.longest })}</span>,
                data.trends.sessions)}
            </div>

            {/* 使用习惯 */}
            <Section title={t("overview:sec.habits")} />
            <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
              {data.bump && (
                <div style={{ flex: "2 1 420px", minWidth: 0 }}>
                  <Card title={t("overview:bump.title")} sub={t("overview:bump.sub")} fill>
                    <div style={{ flex: 1, display: "flex", alignItems: "stretch", minHeight: 200 }}>
                      <Bump bump={data.bump} locale={locale} label={t("overview:bump.aria")} />
                    </div>
                  </Card>
                </div>
              )}
              <div style={{ flex: "1 1 240px", minWidth: 0 }}>
                <Card title={t("overview:clock.title")} sub={t("overview:clock.sub")}>
                  <Clock clock={data.clock} peakHour={data.peakHour} t={t} />
                  <div style={{ marginTop: 12, padding: "8px 10px", borderRadius: 6,
                    background: "var(--acc-soft)", border: "1px solid var(--line6)", fontSize: 11,
                    color: "var(--tx2)", lineHeight: 1.55 }}>
                    <b style={{ color: "var(--tx1)" }}>{t("overview:clock.peakAt", { hour: String(data.peakHour).padStart(2, "0") })}</b>
                    {" "}{t("overview:clock.nightNote", { pct: Math.round(data.nightShare * 100) })}
                  </div>
                </Card>
              </div>
            </div>

            {/* 热力图 */}
            <Card title={t("overview:heat.title")} sub={t("overview:heat.sub", { weeks: data.heatmap.weeks })}
              extra={<span style={{ fontSize: 11, color: "var(--tx4b)", ...num }}>{t("overview:heat.streak", { cur: data.kpis.streak.value, max: data.kpis.streak.longest })}</span>}>
              <div style={{ overflowX: "auto", paddingBottom: 4 }}>
                <Heatmap heatmap={data.heatmap} locale={locale} label={t("overview:heat.aria")} />
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 10, fontSize: 11, color: "var(--tx4b)" }}>
                <span>{t("overview:heat.less")}</span>
                <span style={{ display: "flex", gap: 3, alignItems: "center" }}>
                  {[0, 1, 2, 3, 4].map(i => (
                    <i key={i} style={{ width: 10, height: 10, borderRadius: 2, display: "block",
                      background: `var(--heat-${i})` }} />
                  ))}
                </span>
                <span>{t("overview:heat.more")}</span>
                <div style={{ flex: 1 }} />
                <span style={num}>{t("overview:heat.total", { n: fmtInt(data.heatmap.total) })}</span>
              </div>
            </Card>

            {/* Token 与成本 */}
            <Section title={t("overview:sec.tokens")} />
            <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
              {/* 成本表 */}
              <div style={{ flex: "1 1 340px", minWidth: 0 }}>
                <Card title={t("overview:cost.title")} sub={t("overview:cost.sub")}>
                  <div style={{ display: "flex", alignItems: "baseline", gap: 10 }}>
                    <span style={{ fontSize: 32, fontWeight: 600, letterSpacing: "-0.035em", ...num }}>{fmtCost(data.costTotal)}</span>
                    <span style={{ fontSize: 11, color: "var(--tx4b)" }}>{t("overview:cost.estimate")}</span>
                  </div>
                  <div style={{ overflowX: "auto" }}>
                    <table style={{ width: "100%", borderCollapse: "collapse", marginTop: 14 }}>
                      <thead>
                        <tr>{[t("overview:cost.model"), t("overview:cost.tokens"), t("overview:cost.amount")].map((h, i) => (
                          <th key={i} style={{ fontSize: 10, fontWeight: 600, letterSpacing: ".06em", textTransform: "uppercase",
                            color: "var(--tx4b)", textAlign: i === 0 ? "left" : "right", padding: "0 0 7px", borderBottom: "1px solid var(--line)" }}>{h}</th>
                        ))}</tr>
                      </thead>
                      <tbody>
                        {data.costRows.map((r, i) => (
                          <tr key={r.model}>
                            <td style={{ padding: "8px 0", borderBottom: "1px solid var(--line6)", fontSize: 12,
                              color: "var(--tx1)", display: "flex", alignItems: "center", gap: 7 }}>
                              <i style={{ width: 8, height: 8, borderRadius: 2, flex: "none",
                                background: dotColor(i) }} />{r.model}
                            </td>
                            <td style={{ padding: "8px 0", borderBottom: "1px solid var(--line6)", fontSize: 12,
                              textAlign: "right", color: "var(--tx2)", ...num }}>{fmtTokens(r.total)}</td>
                            <td style={{ padding: "8px 0", borderBottom: "1px solid var(--line6)", fontSize: 12,
                              textAlign: "right", fontWeight: 600, color: "var(--tx1)", ...num }}>{fmtCost(r.cost)}</td>
                          </tr>
                        ))}
                        {data.unpriced.tokens > 0 && (
                          <tr>
                            <td style={{ padding: "8px 0", fontSize: 12, display: "flex", alignItems: "center", gap: 7 }}>
                              <i style={{ width: 8, height: 8, borderRadius: 2, flex: "none", background: "var(--tx4)", opacity: 0.5 }} />
                              <span style={{ color: "var(--tx4b)" }}>{t("overview:cost.unpriced", { n: data.unpriced.models })}</span>
                            </td>
                            <td style={{ padding: "8px 0", fontSize: 12, textAlign: "right", color: "var(--tx4b)", ...num }}>{fmtTokens(data.unpriced.tokens)}</td>
                            <td style={{ padding: "8px 0", fontSize: 12, textAlign: "right", color: "var(--tx5)", fontWeight: 500 }}>—</td>
                          </tr>
                        )}
                      </tbody>
                    </table>
                  </div>
                </Card>
              </div>

              {/* Token 构成 */}
              <div style={{ flex: "1 1 340px", minWidth: 0 }}>
                <Card title={t("overview:comp.title")} sub={t(`overview:scope.${scope}`)} fill>
                  <div style={{ display: "flex", height: 30, borderRadius: 5, overflow: "hidden", gap: 1 }}>
                    {data.composition.filter(c => c.pct > 0).map(c => (
                      <div key={c.key} style={{ flex: c.pct, background: "var(--c1)", opacity: COMP_OPACITY[c.key],
                        display: "grid", placeItems: "center", fontSize: 10, fontWeight: 600, color: "#fff" }}>
                        {c.pct > 12 ? c.pct.toFixed(1) + "%" : ""}
                      </div>
                    ))}
                  </div>
                  <div style={{ marginTop: 14, display: "flex", flexDirection: "column", flex: 1 }}>
                    {data.composition.map((c, i) => (
                      <div key={c.key} style={{ display: "grid", gridTemplateColumns: "1fr auto auto", gap: 12,
                        alignItems: "center", padding: "7px 0", flex: 1,
                        borderTop: i ? "1px solid var(--line6)" : "none" }}>
                        <span style={{ display: "flex", alignItems: "center", gap: 7, color: "var(--tx2)" }}>
                          <i style={{ width: 8, height: 8, borderRadius: 2, background: "var(--c1)", opacity: COMP_OPACITY[c.key] }} />
                          {t(`overview:comp.${c.key}`)}
                        </span>
                        <span style={{ fontSize: 13, fontWeight: 600, ...num }}>{fmtTokens(c.value)}</span>
                        <span style={{ fontSize: 11, color: "var(--tx4b)", minWidth: 42, textAlign: "right", ...num }}>{c.pct.toFixed(1)}%</span>
                      </div>
                    ))}
                  </div>
                </Card>
              </div>
            </div>

            {/* 项目与迁移 */}
            <Section title={t("overview:sec.projects")} />
            <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
              {/* 仓库排行 */}
              <div style={{ flex: "2 1 420px", minWidth: 0 }}>
                <Card title={t("overview:repo.title")} sub={t("overview:repo.sub")}
                  extra={<div style={{ display: "flex", gap: 13, flexWrap: "wrap", fontSize: 11, color: "var(--tx3)" }}>
                    {TOOLS.map(tl => (
                      <span key={tl} style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
                        <i style={{ width: 8, height: 8, borderRadius: 2, background: TOOL_COLOR[tl] || "var(--tx4)" }} />{TOOL_NAME[tl] || tl}
                      </span>
                    ))}
                  </div>}>
                  <div style={{ display: "flex", flexDirection: "column", gap: 9 }}>
                    {data.repos.map(r => {
                      const maxTotal = data.repos[0]?.total || 1;
                      return (
                        <div key={r.name} style={{ display: "grid", gridTemplateColumns: "110px 1fr auto", gap: 10, alignItems: "center" }}>
                          <span title={r.name} style={{ fontSize: 12, color: "var(--tx2)", whiteSpace: "nowrap",
                            overflow: "hidden", textOverflow: "ellipsis" }}>{r.name}</span>
                          <div style={{ height: 7, background: "var(--track)", borderRadius: 4, overflow: "hidden", display: "flex" }}>
                            {TOOLS.map(tl => {
                              const w = (r.byTool[tl] || 0) / maxTotal * 100;
                              return w ? <i key={tl} style={{ display: "block", height: "100%", width: `${w}%`, background: TOOL_COLOR[tl] || "var(--tx4)" }} /> : null;
                            })}
                          </div>
                          <span style={{ fontSize: 11, color: "var(--tx3)", minWidth: 44, textAlign: "right", ...num }}>{fmtInt(r.total)}</span>
                        </div>
                      );
                    })}
                  </div>
                </Card>
              </div>

              {/* 迁移流向 */}
              <div style={{ flex: "1 1 240px", minWidth: 0 }}>
                <Card title={t("overview:flow.title")} sub={t("overview:flow.sub")}>
                  {data.flows.length ? (
                    <div style={{ display: "flex", flexDirection: "column", gap: 7 }}>
                      {data.flows.map((f, i) => {
                        const maxCount = data.flows[0]?.count || 1;
                        return (
                          <div key={i} style={{ display: "grid", gridTemplateColumns: "auto 1fr auto", gap: 10, alignItems: "center",
                            padding: "7px 10px", borderRadius: 6, background: "var(--inset)", border: "1px solid var(--line6)" }}>
                            <span style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--tx2)", whiteSpace: "nowrap" }}>
                              <i style={{ width: 8, height: 8, borderRadius: 2, background: TOOL_COLOR[f.src] || "var(--tx4)" }} />{TOOL_NAME[f.src] || f.src}
                              <span style={{ color: "var(--tx5)" }}>→</span>
                              <i style={{ width: 8, height: 8, borderRadius: 2, background: TOOL_COLOR[f.dst] || "var(--tx4)" }} />{TOOL_NAME[f.dst] || f.dst}
                            </span>
                            <div style={{ height: 5, background: "var(--track)", borderRadius: 3, overflow: "hidden" }}>
                              <i style={{ display: "block", height: "100%", width: `${f.count / maxCount * 100}%`, background: "var(--accent)", opacity: 0.6 }} />
                            </div>
                            <span style={{ fontSize: 12, fontWeight: 600, minWidth: 22, textAlign: "right", ...num }}>{f.count}</span>
                          </div>
                        );
                      })}
                    </div>
                  ) : (
                    <div style={{ padding: "24px 8px", textAlign: "center", color: "var(--tx5)", fontSize: 12 }}>{t("overview:flow.empty")}</div>
                  )}
                </Card>
              </div>
            </div>

            {/* 洞察 */}
            {(featured || smalls.length) && (
              <>
                <Section title={t("overview:sec.insights")} />
                <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
                  {featured && <FeaturedInsight ins={featured} t={t} />}
                  <div style={{ display: "grid", gridTemplateColumns: `repeat(${Math.max(1, smalls.length)}, 1fr)`, gap: 12 }}>
                  {smalls.map((ins, i) => {
                    const c = insightCopy(ins, t);
                    return (
                      <div key={i} style={{ ...card, padding: "15px 16px", display: "flex", flexDirection: "column", gap: 7 }}>
                        <span style={{ fontSize: 10, fontWeight: 600, letterSpacing: ".08em", textTransform: "uppercase", color: "var(--tx4b)" }}>{c.eyebrow}</span>
                        <span style={{ fontSize: 21, fontWeight: 600, letterSpacing: "-0.025em", textWrap: "balance" }}>{c.title}</span>
                        <p style={{ margin: 0, fontSize: 12, color: "var(--tx3)", lineHeight: 1.6 }}>{c.body}</p>
                      </div>
                    );
                  })}
                  </div>
                </div>
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function FeaturedInsight({ ins, t }) {
  const c = insightCopy(ins, t);
  const weeks = ins.params?.weeks;
  const warn = ins.kind === "cost";
  return (
    <div style={{ ...card, display: "flex", flexDirection: "row", gap: 22,
      alignItems: "center", flexWrap: "wrap",
      background: warn ? "linear-gradient(100deg, var(--warn-bg), var(--surface) 62%)" : "var(--surface)",
      borderColor: warn ? "var(--warn-line)" : "var(--line)", padding: "15px 16px" }}>
      <div style={{ flex: "1 1 260px", minWidth: 0, display: "flex", flexDirection: "column", gap: 7 }}>
        <span style={{ fontSize: 10, fontWeight: 600, letterSpacing: ".08em", textTransform: "uppercase",
          color: warn ? "var(--warn)" : "var(--tx4b)" }}>{c.eyebrow}</span>
        <span style={{ fontSize: 24, fontWeight: 600, letterSpacing: "-0.025em", textWrap: "balance" }}>{c.title}</span>
        <p style={{ margin: 0, fontSize: 12, color: "var(--tx3)", lineHeight: 1.6, maxWidth: "54ch" }}>{c.body}</p>
      </div>
      {weeks?.length ? <MiniBars weeks={weeks} label={t("overview:ins.cost.chartLabel", { repo: ins.params.repo })} /> : null}
    </div>
  );
}

// 成本表圆点:按行取分类色板,超出弱化
function dotColor(i) {
  return CHART[i] || "var(--tx4)";
}

