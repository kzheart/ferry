// 其余弹层:差异预览 / 原地修改确认 / 结果 toast /
// 三个筛选弹层 / 快速上手引导(设置与数据来源已合并进 Settings.jsx 全屏页)
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOL_NAME, TOOLS } from "../contracts/tools.js";
import { STATUS_CODE } from "../../modules/migration/migrationModel.js";
import { ACCENT } from "./toolDisplay.js";
import { CloseIcon, SearchIcon, Spinner, ToolIcon } from "./icons.jsx";
import { ConfirmBox } from "./ConfirmBox.jsx";
import {
  FilterCheckRow,
  FilterPopover,
  FilterRadioRow,
  FilterSectionTitle,
} from "./FilterPopover.jsx";

// ---------- 会话搜索命令面板(⌘K 风格居中浮层) ----------
export function SearchPalette({ placeholder, query, onQuery, results,
  recentLabel, emptyLabel, onClose }) {
  const [sel, setSel] = useState(0);
  useEffect(() => setSel(0), [query]);
  useEffect(() => {
    const onKey = e => {
      if (e.key === "Escape") { e.preventDefault(); onClose(); }
      else if (e.key === "ArrowDown") { e.preventDefault(); setSel(s => Math.min(s + 1, results.length - 1)); }
      else if (e.key === "ArrowUp") { e.preventDefault(); setSel(s => Math.max(s - 1, 0)); }
      else if (e.key === "Enter") { e.preventDefault(); results[sel]?.onClick?.(); onClose(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [results, sel, onClose]);
  return (
    <div onClick={onClose}
      style={{ position: "absolute", inset: 0, zIndex: 70, background: "var(--dim)",
        display: "flex", justifyContent: "center", alignItems: "flex-start", paddingTop: "9vh" }}>
      <div onClick={e => e.stopPropagation()} className="fsheet"
        style={{ width: "min(680px, 78vw)", maxHeight: "76vh", display: "flex", flexDirection: "column",
          background: "var(--bg)", borderRadius: 14, boxShadow: "var(--shadow-sheet)", overflow: "hidden" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 11, padding: "0 14px",
          height: 52, borderBottom: "1px solid var(--line5)", flex: "none" }}>
          <span style={{ color: "var(--tx4)", display: "inline-flex" }}><SearchIcon /></span>
          <input autoFocus value={query} onChange={onQuery} placeholder={placeholder}
            style={{ flex: 1, border: "none", background: "transparent", fontSize: 15,
              color: "var(--tx1)", outline: "none" }} />
          <button className="ftool-btn" onClick={onClose}><CloseIcon size={13} /></button>
        </div>
        <div className="fscroll" style={{ overflowY: "auto", padding: "8px", minHeight: 0 }}>
          {recentLabel && (
            <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx4)",
              padding: "6px 10px 4px" }}>{recentLabel}</div>
          )}
          {results.length === 0 ? (
            <div style={{ padding: "26px 12px", textAlign: "center", color: "var(--tx5)",
              fontSize: 13 }}>{emptyLabel}</div>
          ) : results.map((r, i) => (
            <div key={r.id} onMouseEnter={() => setSel(i)}
              onClick={() => { r.onClick?.(); onClose(); }}
              style={{ display: "flex", alignItems: "center", gap: 10, padding: "0 10px", height: 42,
                borderRadius: 8, cursor: "default",
                background: i === sel ? "var(--acc-soft2)" : "transparent" }}>
              {r.tool && <ToolIcon tool={r.tool} size={20} />}
              <span style={{ flex: 1, minWidth: 0, fontSize: 13, color: "var(--tx1)",
                whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{r.title}</span>
              {r.meta && <span className="mono" style={{ fontSize: 11, color: "var(--tx5)", flex: "none",
                maxWidth: "42%", whiteSpace: "nowrap", overflow: "hidden",
                textOverflow: "ellipsis" }}>{r.meta}</span>}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ---------- 删除迁移记录确认 ----------
export function HistoryDeleteConfirm({ h, onCancel, onConfirm }) {
  const { t } = useTranslation();
  return (
    <ConfirmBox width={420} title={t("overlays:historyDelete.title")} actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }}
        onClick={onCancel}>{t("overlays:delete.cancel")}</button>
      <button style={{ height: 34, padding: "0 16px", background: "var(--err2)", border: "none",
        borderRadius: 8, fontSize: 13, color: "#fff", cursor: "default", fontWeight: 600 }}
        onClick={onConfirm}>{t("overlays:historyDelete.confirm")}</button>
    </>}>
      <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 7, lineHeight: 1.5 }}>
        {t("overlays:historyDelete.desc", { title: h.title || h.source_id })}</div>
      <div style={{ marginTop: 14, border: "1px solid var(--line3)", borderRadius: 10,
        padding: "12px 14px", display: "flex", flexDirection: "column", gap: 9 }}>
        {[["var(--ok)", t("overlays:historyDelete.bulletTarget", { tool: TOOL_NAME[h.dst] })],
          ["var(--err)", t("overlays:historyDelete.bulletIrreversible")]].map(([c, txt], i) => (
          <div key={i} style={{ display: "flex", gap: 9, fontSize: 12, color: "var(--tx2b)",
            lineHeight: 1.45 }}>
            <span style={{ width: 5, height: 5, borderRadius: "50%", background: c, flex: "none",
              marginTop: 6 }} />{txt}
          </div>
        ))}
      </div>
    </ConfirmBox>
  );
}

// ---------- 会话右键菜单 ----------
export function ContextMenu({ x, y, items, onClose }) {
  const width = 208;
  const height = items.reduce((a, it) => a + (it.sep ? 9 : 30), 12);
  const left = Math.max(8, Math.min(x, window.innerWidth - width - 8));
  const top = Math.max(8, Math.min(y, window.innerHeight - height - 8));
  return (
    <>
      <div onClick={onClose} onContextMenu={e => { e.preventDefault(); onClose(); }}
        style={{ position: "absolute", inset: 0, zIndex: 55 }} />
      <div style={{ position: "absolute", left, top, width, zIndex: 56, padding: 6,
        background: "var(--bg)", borderRadius: 10,
        boxShadow: "var(--shadow-menu)",
         }}>
        {items.map((it, i) => it.sep
          ? <div key={i} style={{ height: 1, background: "var(--line3)", margin: "4px 8px" }} />
          : (
            <div key={i} className={it.disabled ? undefined : "hov-item"}
              onClick={() => { if (it.disabled) return; onClose(); it.onClick?.(); }}
              title={it.disabled ? it.disabledHint : undefined}
              style={{ display: "flex", alignItems: "center", gap: 8, height: 30,
                padding: "0 9px", borderRadius: 6, fontSize: 12,
                color: it.disabled ? "var(--tx5)" : it.danger ? "var(--err-text)" : "var(--tx2)",
                cursor: it.disabled ? "default" : "pointer", whiteSpace: "nowrap" }}>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>{it.label}</span>
              {it.hint && <span style={{ fontSize: 11, color: "var(--tx5)", flex: "none" }}>{it.hint}</span>}
            </div>
          ))}
      </div>
    </>
  );
}

// ---------- 删除会话确认 ----------
export function SessionDeleteConfirm({ sess, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const subCount = (sess.tree_count || 1) - 1;
  const oc = sess.tool === "opencode";
  const bullets = [
    subCount > 0 && ["var(--warn)", t("overlays:delete.bulletSub", { n: subCount })],
    ["var(--ok)", t("overlays:delete.bulletSnapshot")],
    oc
      ? ["var(--err)", t("overlays:delete.bulletOpenCode")]
      : ["var(--accent)", t("overlays:delete.bulletUndoable")],
  ].filter(Boolean);
  return (
    <ConfirmBox width={430} title={t("overlays:delete.title")} actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>{t("overlays:delete.cancel")}</button>
      <button style={{ height: 34, padding: "0 16px", background: "var(--err2)", border: "none",
        borderRadius: 8, fontSize: 13, color: "#fff", cursor: "default", fontWeight: 600 }}
        onClick={onConfirm}>{oc ? t("overlays:delete.confirmOpenCode") : t("overlays:delete.confirmOther")}</button>
    </>}>
      <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 7, lineHeight: 1.5 }}>
        {t("overlays:delete.desc", { title: sess.title || sess.id, tool: TOOL_NAME[sess.tool] })}</div>
      <div style={{ marginTop: 14, border: "1px solid var(--line3)", borderRadius: 10, padding: "12px 14px",
        display: "flex", flexDirection: "column", gap: 9 }}>
        {bullets.map(([c, txt], i) => (
          <div key={i} style={{ display: "flex", gap: 9, fontSize: 12, color: "var(--tx2b)", lineHeight: 1.45 }}>
            <span style={{ width: 5, height: 5, borderRadius: "50%", background: c, flex: "none",
              marginTop: 6 }} />{txt}
          </div>
        ))}
      </div>
    </ConfirmBox>
  );
}

// ---------- 输入弹框(重命名 / 标签) ----------
export function PromptBox({ title, desc, placeholder, initial, confirmLabel,
  onCancel, onConfirm }) {
  const { t } = useTranslation();
  const [val, setVal] = useState(initial || "");
  const submit = () => onConfirm(val.trim());
  return (
    <ConfirmBox width={420} title={title} actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>{t("overlays:prompt.cancel")}</button>
      <button className="fbtn-primary" style={{ height: 34, padding: "0 16px", fontSize: 13 }}
        onClick={submit}>{confirmLabel || t("overlays:prompt.confirm")}</button>
    </>}>
      {desc && <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 7,
        lineHeight: 1.5 }}>{desc}</div>}
      <input autoFocus value={val} placeholder={placeholder}
        onChange={e => setVal(e.target.value)}
        onKeyDown={e => { if (e.key === "Enter") submit(); }}
        style={{ width: "100%", boxSizing: "border-box", height: 34, marginTop: 12,
          padding: "0 11px", background: "var(--surface)", border: "1px solid var(--line)",
          borderRadius: 8, fontSize: 13, color: "var(--tx1)" }} />
    </ConfirmBox>
  );
}

// ---------- 批量删除确认 ----------
export function BatchDeleteConfirm({ sessions, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const ocCount = sessions.filter(s => s.tool === "opencode").length;
  const bullets = [
    ["var(--ok)", t("overlays:delete.bulletBatchSnapshot")],
    ocCount > 0 && ["var(--err)", t("overlays:delete.bulletBatchOpenCode", { n: ocCount })],
    ["var(--accent)", t("overlays:delete.bulletBatchRest")],
  ].filter(Boolean);
  return (
    <ConfirmBox width={430} title={t("overlays:delete.batchTitle", { n: sessions.length })} actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>{t("overlays:delete.cancel")}</button>
      <button style={{ height: 34, padding: "0 16px", background: "var(--err2)", border: "none",
        borderRadius: 8, fontSize: 13, color: "#fff", cursor: "default", fontWeight: 600 }}
        onClick={onConfirm}>{t("overlays:delete.confirmOther")}</button>
    </>}>
      <div style={{ marginTop: 14, border: "1px solid var(--line3)", borderRadius: 10,
        padding: "12px 14px", display: "flex", flexDirection: "column", gap: 9 }}>
        {bullets.map(([c, txt], i) => (
          <div key={i} style={{ display: "flex", gap: 9, fontSize: 12, color: "var(--tx2b)",
            lineHeight: 1.45 }}>
            <span style={{ width: 5, height: 5, borderRadius: "50%", background: c, flex: "none",
              marginTop: 6 }} />{txt}
          </div>
        ))}
      </div>
    </ConfirmBox>
  );
}

// ---------- 结果 toast ----------
export function Toast({ toast, onDismiss }) {
  const kind = toast.kind;
  const bg = kind === "fail" ? "var(--err-bg)" : kind === "ok" ? "var(--ok-bg)" : "var(--bg)";
  const border = kind === "fail" ? "var(--err-line)" : kind === "ok" ? "var(--ok-line)" : "var(--line3)";
  const color = kind === "fail" ? "var(--err-text)" : kind === "ok" ? "var(--ok-deep)" : "var(--tx2)";
  return (
    <div style={{ position: "absolute", left: "50%", bottom: 26, transform: "translateX(-50%)",
      zIndex: 45, display: "flex", alignItems: "center", gap: 11, padding: "12px 16px",
      borderRadius: 10, background: bg, border: `1px solid ${border}`,
      boxShadow: "var(--shadow-sheet)", maxWidth: 560 }}>
      {kind === "run" ? <Spinner size={20} track="var(--line)" /> : (
        <span style={{ width: 26, height: 26, flex: "none", borderRadius: "50%",
          background: kind === "ok" ? "var(--ok)" : "var(--err)", display: "inline-flex",
          alignItems: "center", justifyContent: "center", color: "#fff", fontSize: 14 }}>
          {kind === "ok" ? "✓" : "×"}</span>)}
      <div>
        <div style={{ fontSize: 13, fontWeight: 600, color }}>{toast.title}</div>
        <div style={{ fontSize: 11, color: "var(--tx3b)", marginTop: 2 }}>{toast.desc}</div>
      </div>
      {toast.action && (
        <button className="fbtn" style={{ height: 28, padding: "0 12px", fontSize: 12,
          flex: "none", fontWeight: 600 }}
          onClick={toast.action.onClick}>{toast.action.label}</button>)}
      <a onClick={onDismiss} style={{ color: "var(--tx5)", fontSize: 16, marginLeft: 6 }}>×</a>
    </div>
  );
}

// 会话库筛选:来源 / 时间 / 目录
export function LibraryFilter({ f, setF, counts, dirs, tags = [], anchor, onClose, onClear }) {
  const { t } = useTranslation();
  const times = [["all", t("overlays:filter.allTime")], ["today", t("overlays:filter.today")],
    ["last7", t("overlays:filter.last7")], ["last30", t("overlays:filter.last30")]];
  return (
    <FilterPopover anchor={anchor} onClose={onClose} onClear={onClear} t={t}>
      <FilterSectionTitle first>{t("overlays:filter.source")}</FilterSectionTitle>
      {TOOLS.map(t2 => (
        <FilterCheckRow key={t2} on={f.src.includes(t2)} icon={<ToolIcon tool={t2} size={24} />}
          label={TOOL_NAME[t2]} extra={counts[t2] || 0}
          onClick={() => setF(v => ({ ...v, src: v.src.includes(t2)
            ? v.src.filter(x => x !== t2) : [...v.src, t2] }))} />
      ))}
      <FilterSectionTitle>{t("overlays:filter.timeRange")}</FilterSectionTitle>
      {times.map(([k, l]) => (
        <FilterRadioRow key={k} on={f.time === k} label={l}
          onClick={() => setF(v => ({ ...v, time: k }))} />
      ))}
      <FilterSectionTitle>{t("overlays:filter.projectDir")}</FilterSectionTitle>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
        {dirs.map(d => {
          const on = f.dir === d;
          return (
            <button key={d} className="mono" onClick={() => setF(v => ({ ...v, dir: on ? null : d }))}
              style={{ height: 24, padding: "0 9px", borderRadius: 20,
                border: `1px solid ${on ? ACCENT : "var(--line)"}`, background: on ? "var(--acc-soft)" : "var(--surface)",
                color: on ? ACCENT : "var(--tx3)", fontSize: 11, cursor: "default" }}>{d}</button>
          );
        })}
        {dirs.length === 0 && <span style={{ fontSize: 11, color: "var(--tx5)" }}>{t("overlays:filter.noDirs")}</span>}
      </div>
      {tags.length > 0 && (<>
        <FilterSectionTitle>{t("overlays:filter.tags")}</FilterSectionTitle>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
          {tags.map(t2 => {
            const on = f.tag === t2;
            return (
              <button key={t2} onClick={() => setF(v => ({ ...v, tag: on ? null : t2 }))}
                style={{ height: 24, padding: "0 9px", borderRadius: 20,
                  border: `1px solid ${on ? ACCENT : "var(--line)"}`,
                  background: on ? "var(--acc-soft)" : "var(--surface)",
                  color: on ? ACCENT : "var(--tx3)", fontSize: 11, cursor: "default" }}>{t2}</button>
            );
          })}
        </div>
      </>)}
      <FilterSectionTitle>{t("overlays:filter.content")}</FilterSectionTitle>
      <FilterCheckRow on={f.mig} label={t("overlays:filter.onlyMigrated")}
        onClick={() => setF(v => ({ ...v, mig: !v.mig }))} />
      <FilterCheckRow on={f.sub} label={t("overlays:filter.onlySubSessions")}
        onClick={() => setF(v => ({ ...v, sub: !v.sub }))} />
    </FilterPopover>
  );
}

// 迁移历史筛选:来源 / 目标 / 状态 / 时间
export function HistoryFilter({ f, setF, anchor, onClose, onClear }) {
  const { t } = useTranslation();
  const statusOptions = [
    [STATUS_CODE.success, t(`common:${STATUS_CODE.success}`)],
    [STATUS_CODE.failed, t(`common:${STATUS_CODE.failed}`)],
    [STATUS_CODE.rolledBack, t(`common:${STATUS_CODE.rolledBack}`)],
  ];
  return (
    <FilterPopover anchor={anchor} onClose={onClose} onClear={onClear} t={t}>
      <FilterSectionTitle first>{t("overlays:filter.sourceTools")}</FilterSectionTitle>
      {TOOLS.map(t2 => (
        <FilterCheckRow key={t2} on={f.src.includes(t2)} icon={<ToolIcon tool={t2} size={24} />}
          label={TOOL_NAME[t2]}
          onClick={() => setF(v => ({ ...v, src: v.src.includes(t2)
            ? v.src.filter(x => x !== t2) : [...v.src, t2] }))} />
      ))}
      <FilterSectionTitle>{t("overlays:filter.targetTool")}</FilterSectionTitle>
      {[["all", t("overlays:filter.allTargets")], ...TOOLS.map(t2 => [t2, TOOL_NAME[t2]])].map(([k, l]) => (
        <FilterRadioRow key={k} on={f.target === k} label={l}
          onClick={() => setF(v => ({ ...v, target: k }))} />
      ))}
      <FilterSectionTitle>{t("overlays:filter.status")}</FilterSectionTitle>
      <FilterRadioRow key="all" on={f.status === "all"} label={t("common:status.all")}
        onClick={() => setF(v => ({ ...v, status: "all" }))} />
      {statusOptions.map(([k, l]) => (
        <FilterRadioRow key={k} on={f.status === k} label={l}
          onClick={() => setF(v => ({ ...v, status: k }))} />
      ))}
      <FilterSectionTitle>{t("overlays:filter.timeRange")}</FilterSectionTitle>
      {[["all", t("overlays:filter.allTime")], ["today", t("overlays:filter.today")],
        ["yesterday", t("overlays:filter.yesterday")], ["earlier", t("overlays:filter.earlier")]].map(([k, l]) => (
        <FilterRadioRow key={k} on={f.time === k} label={l}
          onClick={() => setF(v => ({ ...v, time: k }))} />
      ))}
    </FilterPopover>
  );
}

// ---------- 快速上手引导(coach marks) ----------
const GUIDE_STEPS = [
  { target: "rail", side: "right", titleKey: "onboarding:guide.step1Title", bodyKey: "onboarding:guide.step1Body" },
  { target: "search", side: "right", titleKey: "onboarding:guide.step2Title", bodyKey: "onboarding:guide.step2Body" },
  { target: "scope", side: "top", scroll: true, titleKey: "onboarding:guide.step3Title", bodyKey: "onboarding:guide.step3Body" },
];
const GUIDE_TOTAL = GUIDE_STEPS.length;

export function Guide({ step, onGo, onFinish }) {
  const { t } = useTranslation();
  const [box, setBox] = useState(null);
  const [card, setCard] = useState(null);
  const cfg = GUIDE_STEPS[step - 1];

  useEffect(() => {
    setBox(null);
    const root = document.querySelector("[data-ferry-win]");
    if (!root || !cfg) return;
    const run = () => {
      const el = document.querySelector(`[data-guide="${cfg.target}"]`);
      if (!el) return;
      const w = root.getBoundingClientRect(), r = el.getBoundingClientRect(), pad = 8;
      const W = w.width, H = w.height, cardW = 324;
      const bl = r.left - w.left - pad, bt = Math.max(8, r.top - w.top - pad);
      const bw = r.width + pad * 2, bh = r.height + pad * 2;
      let cl, ct;
      if (cfg.side === "right") { cl = bl + bw + 18; ct = bt; }
      else if (cfg.side === "top") { cl = bl; ct = bt - 198; }
      else { cl = bl + bw - cardW; ct = bt + bh + 16; }
      cl = Math.min(Math.max(12, cl), W - cardW - 12);
      ct = Math.min(Math.max(12, ct), H - 212);
      setBox({ l: bl, t: bt, w: bw, h: bh, W, H });
      setCard({ left: cl, top: ct });
    };
    // 目标在详情区滚动容器内时,先把它滚到视野中段再量位置
    let delay = 30;
    if (cfg.scroll) {
      const sc = document.querySelector("[data-guide-scroll]");
      const el = document.querySelector(`[data-guide="${cfg.target}"]`);
      if (sc && el) {
        const er = el.getBoundingClientRect(), sr = sc.getBoundingClientRect();
        sc.scrollTop += (er.top - sr.top) - 170;
        delay = 80;
      }
    }
    const t = setTimeout(run, delay);
    return () => clearTimeout(t);
  }, [step]);

  if (!cfg) return null;
  const b = box || { l: -9999, t: 0, w: 0, h: 0, W: 4000, H: 3000 };
  const dim = "var(--dim)";
  return (
    <div style={{ position: "absolute", inset: 0, zIndex: 50 }}>
      {box && (
        <>
          <div style={{ position: "absolute", left: 0, top: 0, width: b.W, height: b.t, background: dim }} />
          <div style={{ position: "absolute", left: 0, top: b.t + b.h, width: b.W,
            height: Math.max(0, b.H - b.t - b.h), background: dim }} />
          <div style={{ position: "absolute", left: 0, top: b.t, width: Math.max(0, b.l),
            height: b.h, background: dim }} />
          <div style={{ position: "absolute", left: b.l + b.w, top: b.t,
            width: Math.max(0, b.W - b.l - b.w), height: b.h, background: dim }} />
          <div style={{ position: "absolute", left: b.l, top: b.t, width: b.w, height: b.h,
            borderRadius: 8, outline: `2px solid ${ACCENT}`,
            boxShadow: "0 0 0 4px var(--ring)", pointerEvents: "none",
            transition: "all .26s cubic-bezier(.2,.7,.3,1)" }} />
        </>
      )}
      <div style={{ position: "absolute", left: card?.left ?? -9999, top: card?.top ?? 0, width: 324,
        background: "var(--bg)", borderRadius: 10,
        boxShadow: "var(--shadow-menu)",
        padding: "16px 18px 14px", transition: "all .26s cubic-bezier(.2,.7,.3,1)",
         }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ fontSize: 11, fontWeight: 700, color: ACCENT, letterSpacing: ".03em" }}>
            {step} / {GUIDE_TOTAL}</span>
          <div style={{ display: "flex", gap: 4, marginLeft: 2 }}>
            {GUIDE_STEPS.map((_, idx) => idx + 1).map(i => (
              <span key={i} style={{ width: 16, height: 3, borderRadius: 2,
                background: i <= step ? ACCENT : "var(--dots)" }} />))}
          </div>
          <span style={{ flex: 1 }} />
          <a onClick={onFinish} style={{ fontSize: 11, color: "var(--tx5)" }}>{t("onboarding:guide.skip")}</a>
        </div>
        <div style={{ fontSize: 14, fontWeight: 650, marginTop: 11, letterSpacing: "-.01em" }}>{t(cfg.titleKey)}</div>
        <div style={{ fontSize: 12, color: "var(--tx3)", lineHeight: 1.55, marginTop: 6 }}>{t(cfg.bodyKey)}</div>
        <div style={{ display: "flex", alignItems: "center", gap: 9, marginTop: 15 }}>
          {step > 1 && (
            <button className="fbtn" style={{ height: 31, fontSize: 12 }}
              onClick={() => onGo(step - 1)}>{t("onboarding:guide.back")}</button>)}
          <span style={{ flex: 1 }} />
          <button className="fbtn-primary" style={{ height: 31, padding: "0 16px", fontSize: 12 }}
            onClick={() => step >= GUIDE_TOTAL ? onFinish() : onGo(step + 1)}>
            {step >= GUIDE_TOTAL ? t("onboarding:guide.start") : t("onboarding:guide.next")}</button>
        </div>
      </div>
    </div>
  );
}
