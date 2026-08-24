// 总览页:KPI + 使用习惯 + Token/成本 + 项目/迁移。
// 数据全部由 computeOverview 从真实扫描结果聚合;图表手写内联 SVG,随主题变量着色。
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOLS, TOOL_NAME } from "../../shared/contracts/tools.js";
import {
  formatCompactNumber,
  formatCurrency,
  formatInteger,
} from "../../shared/i18n/numberFormat.js";
import { ToolIcon, SortCaret, CheckIcon, RailGlyph, Spinner } from "../../shared/ui/icons.jsx";
import { computeDayDetail, computeOverview, heatLevel } from "./overviewModel.js";
import { Card, CHART, card, num, Section, toolColor } from "./primitives.jsx";
import TodayPanel from "./TodayPanel.jsx";

const COMP_OPACITY = { cache_read: 0.92, input: 0.6, cache_write: 0.38, output: 0.2 };

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

// 用量走势:按日(7/30 天)或按周(全部)的堆叠柱,按 agent 着色。
// 日桶可点击,与热力图共用"选中日期"联动下钻。
function DailyChart({ daily, metric, locale, label, fmtTokens, fmtCost, t, selectedDay, onPickDay }) {
  const W = 720, H = 200, L = 48, R = 8, T = 14, B = 24;
  const { unit, buckets, tools } = daily;
  const n = buckets.length;
  const max = Math.max(1, metric === "cost" ? daily.maxCost : daily.maxTokens);
  const bw = (W - L - R) / n;
  const gap = Math.min(6, Math.max(2, bw * 0.25));
  const y0 = H - B;
  const hOf = v => (v / max) * (H - T - B);
  const fmt = metric === "cost" ? fmtCost : fmtTokens;
  const clickable = unit === "day" && onPickDay;
  const dayLabel = ts => new Date(ts).toLocaleDateString(locale, { month: "numeric", day: "numeric" });
  const step = Math.max(1, Math.ceil(n / 6));
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height="100%" preserveAspectRatio="xMidYMid meet"
      style={{ display: "block" }} role="img" aria-label={label}>
      {[0.5, 1].map(f => (
        <g key={f}>
          <line x1={L} x2={W - R} y1={y0 - (H - T - B) * f} y2={y0 - (H - T - B) * f}
            stroke="var(--grid)" strokeWidth="1" />
          <text x={L - 7} y={y0 - (H - T - B) * f + 3.5} textAnchor="end" fill="var(--tx5)"
            fontSize="10" fontFamily="var(--font-ui)" style={num}>{fmt(max * f)}</text>
        </g>
      ))}
      <line x1={L} x2={W - R} y1={y0} y2={y0} stroke="var(--line)" strokeWidth="1" />
      {buckets.map((b, i) => {
        const x = L + i * bw + gap / 2, w = Math.max(1, bw - gap);
        const selected = unit === "day" && b.start === selectedDay;
        const dimmed = clickable && selectedDay != null && !selected;
        let acc = 0;
        return (
          <g key={b.start} opacity={dimmed ? 0.4 : 1}
            style={clickable ? { cursor: "pointer" } : undefined}
            onClick={clickable ? () => onPickDay(b.start) : undefined}>
            <title>{`${dayLabel(b.start)} · ${t("overview:today.nSessions", { n: b.sessions })} · ${fmtTokens(b.tokens)} · ${fmtCost(b.cost)}`}</title>
            <rect x={x} y={T} width={w} height={H - T - B} fill="transparent" />
            {tools.map(tl => {
              const v = b.byTool[tl]?.[metric === "cost" ? "cost" : "tokens"] || 0;
              const h = hOf(v);
              if (h < 0.5) return null;
              const y = y0 - hOf(acc) - h;
              acc += v;
              return <rect key={tl} x={x} y={y} width={w} height={Math.max(1, h - 1)}
                rx="1.5" fill={toolColor(tl)} />;
            })}
            {selected && <rect x={x} y={y0 + 3} width={w} height="2.5" rx="1.25" fill="var(--tx1)" />}
          </g>
        );
      })}
      {buckets.map((b, i) => {
        if (unit === "day") {
          if (i % step !== 0) return null;
          return <text key={b.start} x={L + i * bw + bw / 2} y={H - 8} textAnchor="middle"
            fill="var(--tx5)" fontSize="10" fontFamily="var(--font-ui)">{dayLabel(b.start)}</text>;
        }
        const month = new Date(b.start).getMonth();
        const prevMonth = i > 0 ? new Date(buckets[i - 1].start).getMonth() : -1;
        if (i > 0 && month === prevMonth) return null;
        return <text key={b.start} x={L + i * bw + bw / 2} y={H - 8} textAnchor="middle"
          fill="var(--tx5)" fontSize="10" fontFamily="var(--font-ui)">{monthName(month, locale)}</text>;
      })}
    </svg>
  );
}

// 某天明细面板:点击热力图/走势图后展开,展示当天分 agent 与 Top 模型
function DayDetail({ detail, locale, t, fmtInt, fmtTokens, fmtCost, onClose }) {
  const dateStr = new Date(detail.day).toLocaleDateString(locale, {
    month: "long", day: "numeric", weekday: "long",
  });
  const stat = (label, value) => (
    <span style={{ display: "inline-flex", alignItems: "baseline", gap: 6 }}>
      <span style={{ fontSize: 11, color: "var(--tx4b)" }}>{label}</span>
      <span style={{ fontSize: 15, fontWeight: 600, color: "var(--tx1)", ...num }}>{value}</span>
    </span>
  );
  return (
    <div style={{ marginTop: 12, paddingTop: 12, borderTop: "1px solid var(--line6)" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx1)" }}>{dateStr}</span>
        <div style={{ flex: 1 }} />
        <button onClick={onClose} aria-label={t("overview:day.close")}
          style={{ border: "none", background: "none", font: "inherit", fontSize: 11,
            color: "var(--tx4)", cursor: "default", padding: "2px 4px" }}>
          {t("overview:day.close")}
        </button>
      </div>
      {detail.sessions === 0 ? (
        <div style={{ padding: "14px 0 6px", fontSize: 12, color: "var(--tx5)" }}>{t("overview:day.empty")}</div>
      ) : (
        <>
          <div style={{ display: "flex", gap: 22, flexWrap: "wrap", margin: "8px 0 10px" }}>
            {stat(t("overview:day.sessions"), fmtInt(detail.sessions))}
            {stat(t("overview:day.tokens"), fmtTokens(detail.tokens))}
            {stat(t("overview:day.cost"), fmtCost(detail.cost))}
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 5 }}>
            {detail.byAgent.map(a => (
              <div key={a.tool} style={{ display: "grid", gridTemplateColumns: "110px 1fr auto", gap: 10, alignItems: "center" }}>
                <span style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--tx2)" }}>
                  <i style={{ width: 8, height: 8, borderRadius: 2, flex: "none", background: toolColor(a.tool) }} />
                  {TOOL_NAME[a.tool] || a.tool}
                </span>
                <div style={{ height: 6, background: "var(--track)", borderRadius: 3, overflow: "hidden" }}>
                  <i style={{ display: "block", height: "100%", width: `${a.pct}%`, background: toolColor(a.tool) }} />
                </div>
                <span style={{ fontSize: 11, color: "var(--tx3)", minWidth: 96, textAlign: "right", ...num }}>
                  {fmtTokens(a.tokens)} · {a.pct.toFixed(0)}%
                </span>
              </div>
            ))}
          </div>
          {detail.topModels.length > 0 && (
            <div style={{ marginTop: 10, paddingTop: 8, borderTop: "1px solid var(--line6)",
              fontSize: 11, color: "var(--tx4b)", display: "flex", gap: 6, flexWrap: "wrap", alignItems: "baseline" }}>
              <span>{t("overview:day.topModels")}</span>
              {detail.topModels.map((m, i) => (
                <span key={m.model} style={{ color: "var(--tx3)", ...num }}>
                  {i > 0 && <span style={{ color: "var(--tx5)", marginRight: 6 }}>·</span>}
                  {m.model} {m.pct.toFixed(0)}%
                </span>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

function Clock({ clock, peakHour, activeWindow, t }) {
  const CX = 130, CY = 118, R0 = 52, R1 = 88;
  const max = Math.max(1, ...clock);
  const total = clock.reduce((n, v) => n + v, 0);
  const peakLabel = `${String(peakHour).padStart(2, "0")}:00`;

  // 整点对齐:0 点在正上方,6/12/18 依次在右/下/左,和钟面一致
  const ang = h => -Math.PI / 2 + h / 24 * Math.PI * 2;
  const pt = (a, r) => [(CX + Math.cos(a) * r).toFixed(2), (CY + Math.sin(a) * r).toFixed(2)];
  // h0 点到 h1 点的圆弧(h1 可越过 24 表示跨午夜)
  const arc = (h0, h1, r) => {
    const a0 = ang(h0) + 0.02, a1 = ang(h1) - 0.02;
    const [x0, y0] = pt(a0, r), [x1, y1] = pt(a1, r);
    return `M ${x0} ${y0} A ${r} ${r} 0 ${a1 - a0 > Math.PI ? 1 : 0} 1 ${x1} ${y1}`;
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 10 }}>
      <svg viewBox="0 0 260 236" width="100%" height="236" role="img" aria-label={t("overview:clock.aria")}>
        <circle cx={CX} cy={CY} r={R0 - 8} fill="none" stroke="var(--grid)" strokeWidth="1.2" />
        <path d={arc(6, 18, R0 - 8)} fill="none" strokeWidth="3"
          stroke="color-mix(in srgb, var(--warn) 32%, transparent)" />
        {activeWindow && (
          <path d={arc(activeWindow.start, activeWindow.start + activeWindow.len, R1 + 7)}
            fill="none" strokeWidth="2.5" strokeLinecap="round"
            stroke="color-mix(in srgb, var(--c1) 45%, transparent)" />
        )}
        {clock.map((v, h) => {
          if (!v) return null;
          const pct = total ? Math.round(v / total * 100) : 0;
          const peak = h === peakHour;
          const a = ang(h), len = 4 + v / max * (R1 - R0 - 4);
          const [x0, y0] = pt(a, R0), [x1, y1] = pt(a, R0 + len);
          return (
            <line key={h} x1={x0} y1={y0} x2={x1} y2={y1} stroke="var(--c1)"
              opacity={peak ? 1 : 0.5} strokeWidth={peak ? 7 : 6} strokeLinecap="round">
              <title>{t("overview:clock.hourTip", {
                hour: String(h).padStart(2, "0"), count: v, pct,
              })}</title>
            </line>
          );
        })}
        {[0, 3, 6, 9, 12, 15, 18, 21].map(hh => {
          const [x, y] = pt(ang(hh), R1 + 20);
          return (
            <text key={hh} x={x} y={(Number(y) + 3.5).toFixed(1)} textAnchor="middle"
              fill={hh % 6 === 0 ? "var(--tx4)" : "var(--tx5)"} fontSize="9.5"
              fontFamily="var(--font-ui)">{hh}</text>
          );
        })}
        <text x={CX} y={CY - 4} textAnchor="middle" fill="var(--tx1)" fontSize="19" fontWeight="600"
          fontFamily="var(--font-ui)" letterSpacing="-0.5">{total ? peakLabel : "—"}</text>
        {total > 0 && (
          <text x={CX} y={CY + 13} textAnchor="middle" fill="var(--tx4)" fontSize="10"
            fontFamily="var(--font-ui)">{t("overview:clock.peakLabel")}</text>
        )}
      </svg>
      {total > 0 && (
        <div style={{ display: "flex", alignItems: "center", gap: 12, fontSize: 12,
          color: "var(--tx3)", fontFamily: "var(--font-ui)", ...num }}>
          <span>{t("overview:clock.peak")} <b style={{ color: "var(--tx1)", fontWeight: 600 }}>{peakLabel}</b></span>
          {activeWindow && (
            <>
              <span style={{ width: 4, height: 4, borderRadius: "50%", background: "var(--tx5)" }} />
              <span>{t("overview:clock.window")} <b style={{ color: "var(--tx1)", fontWeight: 600 }}>{activeWindow.label}</b></span>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function Heatmap({ heatmap, locale, label, selectedDay, onPickDay }) {
  const { grid, max, start } = heatmap;
  const cell = 11, gap = 3, LX = 26, TY = 8;
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
      {grid.map((col, w) => col.map((c, d) => {
        if (c === -1) return null;
        const day = start + (w * 7 + d) * 86400e3;
        const selected = day === selectedDay;
        return (
          <rect key={`${w}-${d}`} x={LX + w * (cell + gap)} y={TY + d * (cell + gap)}
            width={cell} height={cell} rx="2.5" fill={`var(--heat-${heatLevel(c, max)})`}
            stroke={selected ? "var(--tx1)" : "none"} strokeWidth={selected ? 1.5 : 0}
            style={onPickDay ? { cursor: "pointer" } : undefined}
            onClick={onPickDay ? () => onPickDay(day) : undefined}>
            <title>{c > 0 ? `${c}` : ""}</title>
          </rect>
        );
      }))}
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

export default function Overview({ sessions = [],
  prices = {}, pricing = null, scanning = false, navigationTarget }) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const fmtInt = value => formatInteger(value, locale);
  const fmtTokens = value => formatCompactNumber(value, locale);
  const fmtCost = value => formatCurrency(value, locale);
  const sourceSummary = (pricing?.sources || [])
    .filter(source => source.models > 0)
    .map(source => source.source)
    .join(" · ");
  const pricingUpdated = pricing?.fetched_at
    ? new Date(pricing.fetched_at).toLocaleString(locale, {
      month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
    })
    : "";
  const [scope, setScope] = useState("30");
  const [tool, setTool] = useState("all");
  useEffect(() => {
    if (navigationTarget?.view !== "overview") return;
    if (navigationTarget.agents?.length === 1 &&
        TOOLS.includes(navigationTarget.agents[0])) {
      setTool(navigationTarget.agents[0]);
    }
    const range = navigationTarget.timeRange;
    if (range === "7" || range === "30" || range === "all") setScope(range);
    else if (range && typeof range === "object") {
      const from = Number(range.from);
      const to = Number(range.to || Date.now());
      if (Number.isFinite(from) && Number.isFinite(to) && to >= from) {
        const days = (to - from) / 86_400_000;
        setScope(days <= 7 ? "7" : days <= 30 ? "30" : "all");
      }
    }
  }, [navigationTarget]);
  const data = useMemo(() => computeOverview({
    sessions, prices, scope, tool,
  }), [sessions, prices, scope, tool]);

  // 选中日期:热力图与用量走势共用,点同一天再点一次取消
  const [selectedDay, setSelectedDay] = useState(null);
  const [metric, setMetric] = useState("tokens");
  const pickDay = day => setSelectedDay(cur => (cur === day ? null : day));
  useEffect(() => { setSelectedDay(null); }, [tool]);
  const dayDetail = useMemo(() => {
    if (selectedDay == null) return null;
    const filtered = tool === "all" ? sessions : sessions.filter(s => s.tool === tool);
    return computeDayDetail({ sessions: filtered, prices, day: selectedDay });
  }, [selectedDay, sessions, prices, tool]);

  const delta = (kpi, fmt) => {
    if (kpi.delta == null) return null;
    const up = kpi.delta >= 0;
    return <span style={{ fontSize: 11, fontWeight: 500, color: up ? "var(--ok)" : "var(--err)", ...num }}>
      {up ? "+" : "−"}{fmt(Math.abs(kpi.delta))} {t("overview:kpi.thisPeriod")}
    </span>;
  };

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
            <TodayPanel today={data.today} streak={data.kpis.streak.value} locale={locale} t={t}
              fmtInt={fmtInt} fmtTokens={fmtTokens} fmtCost={fmtCost} />

            <div data-guide="overview-kpis"
              style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))", gap: 12 }}>
              {kpiCard(t("overview:kpi.sessions"), fmtInt(data.kpis.sessions.value), null,
                delta(data.kpis.sessions, fmtInt), data.trends.sessions)}
              {kpiCard(t("overview:kpi.tokens"), fmtTokens(data.kpis.tokens.value), null,
                delta(data.kpis.tokens, v => fmtTokens(v)), data.trends.tokens)}
              {kpiCard(t("overview:kpi.cost"), fmtCost(data.kpis.cost.value), null,
                delta(data.kpis.cost, fmtCost), data.trends.cost)}
              {kpiCard(t("overview:kpi.streak"), data.kpis.streak.value, t("overview:kpi.days"),
                <span style={{ fontSize: 11, color: "var(--tx4b)", ...num }}>{t("overview:kpi.longest", { n: data.kpis.streak.longest })}</span>,
                data.trends.sessions)}
            </div>

            <Section title={t("overview:sec.habits")} />
            <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
              {data.daily.tools.length > 0 && (
                <div style={{ flex: "2 1 420px", minWidth: 0 }}>
                  <Card title={t("overview:daily.title")}
                    sub={data.daily.unit === "day"
                      ? t("overview:daily.subDay", { range: t(`overview:scope.${scope}`) })
                      : (data.daily.truncated ? t("overview:daily.subWeekCapped") : t("overview:daily.subWeek"))}
                    extra={
                      <div role="group" aria-label={t("overview:daily.metric")}
                        style={{ display: "flex", background: "var(--track)", borderRadius: 6, padding: 2, gap: 2 }}>
                        {segBtn(t("overview:daily.tokens"), metric === "tokens", () => setMetric("tokens"))}
                        {segBtn(t("overview:daily.cost"), metric === "cost", () => setMetric("cost"))}
                      </div>
                    } fill>
                    <div style={{ flex: 1, display: "flex", alignItems: "stretch", minHeight: 200 }}>
                      <DailyChart daily={data.daily} metric={metric} locale={locale}
                        label={t("overview:daily.aria")} fmtTokens={fmtTokens} fmtCost={fmtCost} t={t}
                        selectedDay={selectedDay} onPickDay={pickDay} />
                    </div>
                    <div style={{ display: "flex", gap: 13, flexWrap: "wrap", fontSize: 11, color: "var(--tx3)", marginTop: 6 }}>
                      {data.daily.tools.map(tl => (
                        <span key={tl} style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
                          <i style={{ width: 8, height: 8, borderRadius: 2, background: toolColor(tl) }} />{TOOL_NAME[tl] || tl}
                        </span>
                      ))}
                    </div>
                  </Card>
                </div>
              )}
              <div style={{ flex: "1 1 240px", minWidth: 0 }}>
                <Card title={t("overview:clock.title")} sub={t("overview:clock.sub")}>
                  <Clock clock={data.clock} peakHour={data.peakHour} activeWindow={data.activeWindow} t={t} />
                </Card>
              </div>
            </div>

            <Card title={t("overview:heat.title")} sub={t("overview:heat.sub", { weeks: data.heatmap.weeks })}
              extra={<span style={{ fontSize: 11, color: "var(--tx4b)", ...num }}>{t("overview:heat.streak", { cur: data.kpis.streak.value, max: data.kpis.streak.longest })}</span>}>
              <div style={{ overflowX: "auto", paddingBottom: 4 }}>
                <Heatmap heatmap={data.heatmap} locale={locale} label={t("overview:heat.aria")}
                  selectedDay={selectedDay} onPickDay={pickDay} />
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
              {dayDetail && (
                <DayDetail detail={dayDetail} locale={locale} t={t} fmtInt={fmtInt}
                  fmtTokens={fmtTokens} fmtCost={fmtCost} onClose={() => setSelectedDay(null)} />
              )}
            </Card>

            <Section title={t("overview:sec.tokens")} />
            <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
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
                  {(sourceSummary || pricingUpdated) && (
                    <div style={{ marginTop: 10, paddingTop: 9, borderTop: "1px solid var(--line6)",
                      display: "flex", gap: 8, flexWrap: "wrap", fontSize: 10, color: "var(--tx4b)" }}>
                      {sourceSummary && <span>{t("overview:cost.sources", { sources: sourceSummary })}</span>}
                      {pricingUpdated && <span>{t("overview:cost.updated", { time: pricingUpdated })}</span>}
                    </div>
                  )}
                </Card>
              </div>

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

            <Section title={t("overview:sec.projects")} />
            <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
              <div style={{ flex: "2 1 420px", minWidth: 0 }}>
                <Card title={t("overview:repo.title")} sub={t("overview:repo.sub")} fill>
                  {/* 行 flex:1 均摊高度,与右侧 Agent 对比卡等高时行距自适应 */}
                  <div style={{ display: "flex", flexDirection: "column", flex: 1, gap: 9 }}>
                    {data.repos.map(r => {
                      const maxTotal = data.repos[0]?.total || 1;
                      return (
                        <div key={r.name} style={{ display: "grid", gridTemplateColumns: "110px 1fr auto", gap: 10,
                          alignItems: "center", flex: 1 }}>
                          <span title={r.name} style={{ fontSize: 12, color: "var(--tx2)", whiteSpace: "nowrap",
                            overflow: "hidden", textOverflow: "ellipsis" }}>{r.name}</span>
                          <div style={{ height: 7, background: "var(--track)", borderRadius: 4, overflow: "hidden", display: "flex" }}>
                            {TOOLS.map(tl => {
                              const w = (r.byTool[tl] || 0) / maxTotal * 100;
                              return w ? <i key={tl} style={{ display: "block", height: "100%", width: `${w}%`, background: toolColor(tl) }} /> : null;
                            })}
                          </div>
                          <span style={{ fontSize: 11, color: "var(--tx3)", minWidth: 44, textAlign: "right", ...num }}>{fmtInt(r.total)}</span>
                        </div>
                      );
                    })}
                  </div>
                  <div style={{ display: "flex", gap: 13, flexWrap: "wrap", fontSize: 11, color: "var(--tx3)", marginTop: 12 }}>
                    {TOOLS.map(tl => (
                      <span key={tl} style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
                        <i style={{ width: 8, height: 8, borderRadius: 2, background: toolColor(tl) }} />{TOOL_NAME[tl] || tl}
                      </span>
                    ))}
                  </div>
                </Card>
              </div>

              <div style={{ flex: "1 1 240px", minWidth: 0 }}>
                <Card title={t("overview:agents.title")}
                  sub={t("overview:agents.sub", { range: t(`overview:scope.${scope}`) })} fill>
                  {data.agentShare.length ? (
                    <div style={{ display: "flex", flexDirection: "column", flex: 1 }}>
                      {data.agentShare.map((a, i) => (
                        <div key={a.tool} style={{ display: "grid", gridTemplateColumns: "1fr auto", gap: 12,
                          alignItems: "center", padding: "8px 0", flex: 1,
                          borderTop: i ? "1px solid var(--line6)" : "none" }}>
                          <div style={{ minWidth: 0 }}>
                            <div style={{ display: "flex", alignItems: "center", gap: 7, fontSize: 12, color: "var(--tx2)", marginBottom: 5 }}>
                              <ToolIcon tool={a.tool} size={16} />
                              <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{TOOL_NAME[a.tool] || a.tool}</span>
                              <span style={{ fontSize: 11, color: "var(--tx4b)", ...num }}>{fmtTokens(a.tokens)}</span>
                            </div>
                            <div style={{ height: 6, background: "var(--track)", borderRadius: 3, overflow: "hidden" }}>
                              <i style={{ display: "block", height: "100%", width: `${a.pct}%`, background: toolColor(a.tool) }} />
                            </div>
                          </div>
                          <div style={{ textAlign: "right", minWidth: 74 }}>
                            <div style={{ fontSize: 13, fontWeight: 600, ...num }}>{a.pct.toFixed(0)}%</div>
                            {a.delta != null && (
                              <div style={{ fontSize: 11, fontWeight: 500, ...num,
                                color: a.delta >= 0 ? "var(--ok)" : "var(--err)" }}>
                                {a.delta >= 0 ? "+" : "−"}{Math.abs(a.delta).toFixed(1)} {t("overview:agents.pp")}
                              </div>
                            )}
                          </div>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div style={{ padding: "24px 8px", textAlign: "center", color: "var(--tx5)", fontSize: 12 }}>{t("overview:agents.empty")}</div>
                  )}
                </Card>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// 成本表圆点:按行取分类色板,超出弱化
function dotColor(i) {
  return CHART[i] || "var(--tx4)";
}
