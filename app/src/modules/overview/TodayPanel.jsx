// 今日速览:一行指标 + 悬浮明细。数值下方不再堆说明文案,细项全部收进悬浮层。
// 不受时间范围分段器影响(“今天”本身就是范围),但沿用页面顶部的 agent 筛选。
import { useState } from "react";
import { TOOL_NAME } from "../../shared/contracts/tools.js";
import { Card, num, toolColor } from "./primitives.jsx";

const TOP_N = 5;   // 悬浮层最多列这么多行,其余折叠成一行汇总

// --tooltip 在亮/暗两套主题下都是深底(与 AskFerry 的浮层同款),所以层内文字固定走
// 白色系而不是 --tx* 文本变量——后者在暗色下会翻成深色,叠在深底上直接不可读。
const TIP_FG = "#FFFFFF";
const TIP_MUT = "rgba(255,255,255,.58)";
const TIP_LINE = "rgba(255,255,255,.14)";

// 悬浮层:鼠标悬停与键盘 focus 都能唤出,避免明细只有鼠标可达
function Metric({ label, children, tip, align = "left" }) {
  const [open, setOpen] = useState(false);
  const show = () => setOpen(true);
  const hide = () => setOpen(false);
  return (
    <div style={{ position: "relative", minWidth: 0, outlineOffset: 3 }}
      tabIndex={tip ? 0 : undefined}
      onMouseEnter={show} onMouseLeave={hide} onFocus={show} onBlur={hide}>
      {/* 虚线下划线是这一格“可悬停看明细”的唯一暗示,值下方不再挂说明文案 */}
      <span style={{ fontSize: 11, color: open ? "var(--tx3)" : "var(--tx4b)",
        textDecoration: tip ? "underline dotted var(--tx5)" : undefined, textUnderlineOffset: 3 }}>{label}</span>
      {children}
      {tip && open && (
        <div role="tooltip" style={{ position: "absolute", zIndex: 40, top: "calc(100% + 8px)",
          [align]: 0, minWidth: 190, maxWidth: 280, padding: "9px 12px 10px", borderRadius: 8,
          background: "var(--tooltip)", color: TIP_FG, boxShadow: "var(--shadow-menu)",
          fontSize: 11.5, lineHeight: 1.5, pointerEvents: "none" }}>
          {tip}
        </div>
      )}
    </div>
  );
}

const TipTitle = ({ children }) => (
  <div style={{ fontSize: 10.5, color: TIP_MUT, marginBottom: 6 }}>{children}</div>
);

const TipRow = ({ dot, name, mono, value, pct, dim }) => (
  <div style={{ display: "flex", alignItems: "baseline", gap: 8, padding: "1.5px 0",
    color: dim ? TIP_MUT : TIP_FG }}>
    {dot && <i style={{ width: 7, height: 7, borderRadius: 2, background: dot, flex: "none", alignSelf: "center" }} />}
    <span style={{ flex: 1, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis",
      fontFamily: mono ? "var(--font-mono)" : undefined, fontSize: mono ? 11 : undefined }}>{name}</span>
    {value != null && <span style={num}>{value}</span>}
    {pct != null && <span style={{ ...num, color: TIP_MUT, width: 34, textAlign: "right" }}>{pct}</span>}
  </div>
);

const TipSplit = () => (
  <div style={{ borderTop: `1px solid ${TIP_LINE}`, margin: "6px 0" }} />
);

// 列表折叠:超出 TOP_N 的行并成一条,避免长尾把悬浮层撑高
function fold(rows, sum) {
  if (rows.length <= TOP_N) return { head: rows, rest: null };
  const tail = rows.slice(TOP_N);
  return { head: rows.slice(0, TOP_N), rest: { count: tail.length, value: tail.reduce(sum, 0) } };
}

export default function TodayPanel({ today, streak, locale, t, fmtInt, fmtTokens, fmtCost }) {
  const dateLabel = new Date(today.day).toLocaleDateString(locale, {
    month: "long", day: "numeric", weekday: "short",
  });
  const timeLabel = new Date(today.asOf).toLocaleTimeString(locale, {
    hour: "2-digit", minute: "2-digit",
  });
  // 昨日同时段基数极小时百分比会飙到四五位数,截顶,否则数字会把这一格撑破
  const pctLabel = value => {
    const abs = Math.abs(value);
    return `${value >= 0 ? "↑" : "↓"} ${abs > 999 ? ">999" : abs.toFixed(0)}%`;
  };

  const badge = (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 7, fontSize: 11,
      color: "var(--tx3b)", background: "var(--inset)", border: "1px solid var(--line6)",
      borderRadius: 999, padding: "3px 10px", whiteSpace: "nowrap" }}>
      <span style={{ display: "inline-flex", gap: 2.5, alignItems: "center" }}
        role="img" aria-label={t("overview:today.weekAria")}>
        {today.week.map(d => (
          <i key={d.day} title={new Date(d.day).toLocaleDateString(locale, { month: "numeric", day: "numeric" })}
            style={{ width: 7, height: 7, borderRadius: 2, display: "block",
              background: d.level ? `var(--heat-${d.level})` : "var(--track)" }} />
        ))}
      </span>
      {t("overview:today.streak", { n: streak })}
    </span>
  );

  if (today.empty) {
    return (
      <Card title={t("overview:today.title")} sub={dateLabel} extra={badge}>
        <div style={{ fontSize: 13, color: "var(--tx4)", padding: "2px 0" }}>
          {streak > 0 ? t("overview:today.emptyStreak", { n: streak }) : t("overview:today.empty")}
        </div>
      </Card>
    );
  }

  const agents = fold(today.byAgent, (n, r) => n + r.tokens);
  const models = fold(today.byModel, (n, r) => n + r.tokens);
  const costs = fold(today.costByModel, (n, r) => n + r.cost);
  const projects = fold(today.byProject, (n, r) => n + r.sessions);
  const { current, yesterday, tokensPct, costPct } = today.compare;

  const value = (text, color) => (
    <div style={{ fontSize: 21, fontWeight: 600, letterSpacing: "-0.02em", lineHeight: 1.2,
      marginTop: 2, color: color || "var(--tx1)", ...num }}>{text}</div>
  );
  const summary = stat => `${fmtTokens(stat.tokens)} · ${fmtCost(stat.cost)} · ${t("overview:today.nSessions", { n: fmtInt(stat.sessions) })}`;

  return (
    <Card title={t("overview:today.title")} sub={dateLabel} extra={badge}>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(168px, 1fr))", gap: 18 }}>

        <Metric label={t("overview:today.sessions")} tip={
          <>
            <TipTitle>{t("overview:today.projects")}</TipTitle>
            {projects.head.map(p => (
              <TipRow key={p.name} name={p.name} mono
                value={t("overview:today.nSessions", { n: fmtInt(p.sessions) })} />
            ))}
            {projects.rest && <TipRow dim name={t("overview:today.other", { n: projects.rest.count })}
              value={t("overview:today.nSessions", { n: fmtInt(projects.rest.value) })} />}
            {!today.byProject.length && <TipRow dim name={t("overview:today.noProject")} />}
            <TipSplit />
            <TipRow dim name={t("overview:today.createdNote", { n: fmtInt(today.created) })} />
          </>
        }>
          {value(fmtInt(today.sessions))}
        </Metric>

        <Metric label={t("overview:today.tokens")} tip={
          <>
            <TipTitle>{t("overview:today.byAgent")}</TipTitle>
            {agents.head.map(a => (
              <TipRow key={a.tool} dot={toolColor(a.tool)} name={TOOL_NAME[a.tool] || a.tool}
                value={fmtTokens(a.tokens)} pct={`${a.pct.toFixed(0)}%`} />
            ))}
            {agents.rest && <TipRow dim name={t("overview:today.other", { n: agents.rest.count })}
              value={fmtTokens(agents.rest.value)} />}
            <TipSplit />
            <TipRow dim name={today.composition.map(c => t(`overview:comp.${c.key}`)).join(" / ")} />
            <TipRow dim name={today.composition.map(c => fmtTokens(c.value)).join(" / ")} />
          </>
        }>
          {value(fmtTokens(today.tokens))}
          <div style={{ display: "flex", height: 5, borderRadius: 3, overflow: "hidden",
            marginTop: 7, maxWidth: 130, background: "var(--track)" }}>
            {today.byAgent.map(a => (
              <i key={a.tool} style={{ display: "block", height: "100%", width: `${a.pct}%`,
                background: toolColor(a.tool) }} />
            ))}
          </div>
        </Metric>

        <Metric label={t("overview:today.cost")} tip={
          <>
            <TipTitle>{t("overview:today.costByModel")}</TipTitle>
            {costs.head.map(r => (
              <TipRow key={r.model} name={r.model} mono value={fmtCost(r.cost)} />
            ))}
            {costs.rest && <TipRow dim name={t("overview:today.other", { n: costs.rest.count })}
              value={fmtCost(costs.rest.value)} />}
            {!today.costByModel.length && <TipRow dim name={t("overview:today.noPriced")} />}
            <TipSplit />
            <TipRow dim name={t("overview:today.costNote")} />
          </>
        }>
          {value(fmtCost(today.cost))}
        </Metric>

        <Metric label={t("overview:today.compare")} tip={
          <>
            <TipTitle>{t("overview:today.asOf", { time: timeLabel })}</TipTitle>
            <TipRow name={t("overview:today.todayRow")} value={summary(current)} />
            <TipRow name={t("overview:today.yesterdayRow")} value={summary(yesterday)} />
            <TipSplit />
            <TipRow name={t("overview:today.tokensRow")}
              value={tokensPct == null ? "—" : pctLabel(tokensPct)} />
            <TipRow name={t("overview:today.costRow")}
              value={costPct == null ? "—" : pctLabel(costPct)} />
          </>
        }>
          {tokensPct == null
            ? value("—", "var(--tx4)")
            : value(pctLabel(tokensPct), tokensPct >= 0 ? "var(--ok)" : "var(--err)")}
        </Metric>

        <Metric align="right" label={t("overview:today.topModel")} tip={
          <>
            <TipTitle>{t("overview:today.byModel")}</TipTitle>
            {models.head.map(m => (
              <TipRow key={m.model} name={m.model} mono value={fmtTokens(m.tokens)}
                pct={`${m.pct.toFixed(0)}%`} />
            ))}
            {models.rest && <TipRow dim name={t("overview:today.other", { n: models.rest.count })}
              value={fmtTokens(models.rest.value)} />}
          </>
        }>
          <div style={{ fontSize: 14, fontWeight: 600, color: "var(--tx2)", lineHeight: 1.9,
            fontFamily: "var(--font-mono)", whiteSpace: "nowrap", overflow: "hidden",
            textOverflow: "ellipsis" }} title={today.topModel || ""}>
            {today.topModel || "—"}
          </div>
        </Metric>

      </div>
    </Card>
  );
}
