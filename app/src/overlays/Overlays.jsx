// 其余弹层:差异预览 / 原地修改确认 / 快照还原确认 / 结果 toast /
// 三个筛选弹层 / 快速上手引导(设置与数据来源已合并进 Settings.jsx 全屏页)
import { useEffect, useState } from "react";
import { ACCENT, TOOL_NAME, TOOLS, fmtSize, fmtTime } from "../api.js";
import { Spinner, ToolIcon } from "../icons.jsx";
import { CheckSquare, RadioDot, Sheet } from "../components/ui.jsx";

// ---------- 差异预览 ----------
export function DiffSheet({ ops, preview, loading, onClose }) {
  return (
    <Sheet width={760} maxHeight={780} onClose={onClose}>
      <div style={{ flex: "none", padding: "15px 20px", borderBottom: "1px solid var(--line5)",
        display: "flex", alignItems: "center" }}>
        <div style={{ fontSize: 14.5, fontWeight: 650 }}>差异预览</div>
        <div style={{ fontSize: 12, color: "var(--tx4)", marginLeft: 12 }}>
          {ops.length} 项暂存操作
          {preview && ` · ${fmtSize(preview.before.size)} → ${fmtSize(preview.after.size)}
            · ${preview.before.count} → ${preview.after.count} 条记录`}
        </div>
        <div style={{ flex: 1 }} />
        <a onClick={onClose} style={{ color: "var(--tx5)", fontSize: 18 }}>×</a>
      </div>
      <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "18px 20px" }}>
        {ops.length === 0 && (
          <div style={{ textAlign: "center", color: "var(--tx5)", fontSize: 13, padding: 40 }}>尚无暂存操作</div>)}
        {loading && (
          <div style={{ display: "flex", alignItems: "center", gap: 10, color: "var(--tx4)",
            fontSize: 12.5, marginBottom: 14 }}><Spinner size={14} /> 正在计算前后差异…</div>)}
        {ops.map(o => (
          <div key={o.id} style={{ border: "1px solid var(--line3)", borderRadius: 10, overflow: "hidden",
            marginBottom: 12 }}>
            <div style={{ padding: "9px 13px", background: "var(--fill2)", borderBottom: "1px solid var(--line5)",
              display: "flex", alignItems: "center", gap: 9 }}>
              <span style={{ width: 7, height: 7, borderRadius: "50%", background: o.dot }} />
              <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx2)" }}>{o.label}</span>
              <span style={{ fontSize: 11, color: "var(--tx5)", marginLeft: "auto" }}>{o.delta}</span>
            </div>
            <div className="mono" style={{ padding: "11px 13px", fontSize: 11.5, lineHeight: 1.7 }}>
              <div style={{ background: "var(--err-bg2)", color: "var(--err-text)", padding: "2px 8px", borderRadius: 5,
                marginBottom: 4, textDecoration: o.type === "delete" ? "line-through" : "none",
                whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>− {o.before}</div>
              {o.after && (
                <div style={{ background: "var(--ok-bg2)", color: "var(--ok-body2)", padding: "2px 8px", borderRadius: 5,
                  whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>+ {o.after}</div>)}
            </div>
          </div>
        ))}
        {preview?.notes?.length > 0 && (
          <div style={{ fontSize: 12, color: "var(--tx3b)", lineHeight: 1.6 }}>
            引擎确认:{preview.notes.join(";")}</div>)}
      </div>
      <div style={{ flex: "none", padding: "13px 20px", borderTop: "1px solid var(--line5)",
        display: "flex", justifyContent: "flex-end" }}>
        <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onClose}>关闭</button>
      </div>
    </Sheet>
  );
}

// ---------- 小确认框 ----------
function ConfirmBox({ width = 400, title, children, actions }) {
  return (
    <div style={{ position: "absolute", inset: 0, background: "var(--scrim)", display: "flex",
      alignItems: "center", justifyContent: "center", zIndex: 44, animation: "ffade .15s ease" }}>
      <div style={{ width, background: "var(--bg)", borderRadius: 12,
        boxShadow: "0 24px 60px -18px rgba(20,28,38,.5)", padding: 22, animation: "fsheet .2s ease" }}>
        <div style={{ fontSize: 15, fontWeight: 650 }}>{title}</div>
        {children}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 10, marginTop: 18 }}>{actions}</div>
      </div>
    </div>
  );
}

export function InplaceConfirm({ onCancel, onConfirm }) {
  return (
    <ConfirmBox title="原地修改原会话?" actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>取消</button>
      <button style={{ height: 34, padding: "0 16px", background: "var(--err2)", border: "none",
        borderRadius: 8, fontSize: 13, color: "#fff", cursor: "pointer", fontWeight: 600 }}
        onClick={onConfirm}>确认原地修改</button>
    </>}>
      <div style={{ fontSize: 12.5, color: "var(--tx3b)", marginTop: 8, lineHeight: 1.55 }}>
        这会直接改写原始会话文件。Ferry 会先自动创建快照;若应用后验收未通过将自动还原。
        验收默认只做结构验证;可在设置中开启运行时探针,探针只在临时影子会话上执行,不会向原会话追加消息。此操作可通过快照撤销。</div>
    </ConfirmBox>
  );
}

export function SnapRestoreConfirm({ snap, onCancel, onConfirm }) {
  const bullets = [
    ["var(--warn)", "当前会话在此快照之后的改动将被覆盖。"],
    ["var(--ok)", "Ferry 会在还原前自动创建一个当前状态的保护快照。"],
    ["var(--accent)", "还原完成后可通过该保护快照撤销本次操作。"],
    ["var(--info-dot)", "源工具中的其他会话不受影响。"],
  ];
  return (
    <ConfirmBox width={440} title="还原到此快照?" actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>取消</button>
      <button className="fbtn-primary" style={{ height: 34, padding: "0 16px", fontSize: 13 }}
        onClick={onConfirm}>创建保护快照并还原</button>
    </>}>
      <div style={{ fontSize: 12.5, color: "var(--tx3b)", marginTop: 7, lineHeight: 1.5 }}>
        会话「{snap.title}」将恢复到 {fmtTime(snap.time)} 的状态。</div>
      <div style={{ marginTop: 14, border: "1px solid var(--line3)", borderRadius: 10, padding: "12px 14px",
        display: "flex", flexDirection: "column", gap: 9 }}>
        {bullets.map(([c, t], i) => (
          <div key={i} style={{ display: "flex", gap: 9, fontSize: 12, color: "var(--tx2b)", lineHeight: 1.45 }}>
            <span style={{ width: 5, height: 5, borderRadius: "50%", background: c, flex: "none",
              marginTop: 6 }} />{t}
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
      boxShadow: "0 12px 30px -12px rgba(20,28,38,.4)", animation: "fsheet .22s ease", maxWidth: 560 }}>
      {kind === "run" ? <Spinner size={20} track="var(--line)" /> : (
        <span style={{ width: 26, height: 26, flex: "none", borderRadius: "50%",
          background: kind === "ok" ? "var(--ok)" : "var(--err)", display: "inline-flex",
          alignItems: "center", justifyContent: "center", color: "#fff", fontSize: 14 }}>
          {kind === "ok" ? "✓" : "×"}</span>)}
      <div>
        <div style={{ fontSize: 13, fontWeight: 600, color }}>{toast.title}</div>
        <div style={{ fontSize: 11.5, color: "var(--tx3b)", marginTop: 2 }}>{toast.desc}</div>
      </div>
      <a onClick={onDismiss} style={{ color: "var(--tx5)", fontSize: 16, marginLeft: 6 }}>×</a>
    </div>
  );
}

// ---------- 筛选弹层(共用外壳) ----------
function PopShell({ onClose, onClear, children }) {
  return (
    <>
      <div onClick={onClose} style={{ position: "absolute", inset: 0, zIndex: 35 }} />
      <div style={{ position: "absolute", left: 66, top: 190, width: 272, zIndex: 36,
        background: "var(--bg)", borderRadius: 11,
        boxShadow: "0 16px 40px -14px rgba(20,28,38,.42),0 0 0 1px var(--ring)",
        overflow: "hidden", animation: "fpop .14s ease" }}>
        <div className="fscroll" style={{ maxHeight: 430, overflowY: "auto", padding: "12px 13px" }}>
          {children}
        </div>
        <div style={{ display: "flex", alignItems: "center", padding: "9px 13px",
          borderTop: "1px solid var(--line5)" }}>
          <a onClick={onClear} style={{ fontSize: 11.5, color: "var(--tx3b)" }}>清除筛选</a>
          <span style={{ flex: 1 }} />
          <button className="fbtn-primary" style={{ height: 28, padding: "0 14px", fontSize: 12 }}
            onClick={onClose}>完成</button>
        </div>
      </div>
    </>
  );
}

const SectionTitle = ({ children, first }) => (
  <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx5)", letterSpacing: ".03em",
    margin: first ? "0 0 6px" : "12px 0 6px" }}>{children}</div>
);

function CheckRow({ on, onClick, icon, label, extra }) {
  return (
    <div className="hov-item" onClick={onClick}
      style={{ display: "flex", alignItems: "center", gap: 9, padding: "6px 7px", borderRadius: 7,
        cursor: "pointer" }}>
      <CheckSquare on={on} />
      {icon}
      <span style={{ fontSize: 12.5, color: "var(--tx2)", flex: 1, whiteSpace: "nowrap",
        overflow: "hidden", textOverflow: "ellipsis" }}>{label}</span>
      {extra && <span style={{ fontSize: 11, color: "var(--tx5)" }}>{extra}</span>}
    </div>
  );
}

function RadioRow({ on, onClick, label }) {
  return (
    <div className="hov-item" onClick={onClick}
      style={{ display: "flex", alignItems: "center", gap: 9, padding: "6px 7px", borderRadius: 7,
        cursor: "pointer" }}>
      <RadioDot on={on} />
      <span style={{ fontSize: 12.5, color: "var(--tx2)", whiteSpace: "nowrap", overflow: "hidden",
        textOverflow: "ellipsis" }}>{label}</span>
    </div>
  );
}

// 会话库筛选:来源 / 时间 / 目录
export function LibraryFilter({ f, setF, counts, dirs, onClose, onClear }) {
  const times = [["all", "全部时间"], ["today", "今天"], ["last7", "最近 7 天"], ["last30", "最近 30 天"]];
  return (
    <PopShell onClose={onClose} onClear={onClear}>
      <SectionTitle first>来源</SectionTitle>
      {TOOLS.map(t => (
        <CheckRow key={t} on={f.src.includes(t)} icon={<ToolIcon tool={t} size={24} />}
          label={TOOL_NAME[t]} extra={counts[t] || 0}
          onClick={() => setF(v => ({ ...v, src: v.src.includes(t)
            ? v.src.filter(x => x !== t) : [...v.src, t] }))} />
      ))}
      <SectionTitle>时间范围</SectionTitle>
      {times.map(([k, l]) => (
        <RadioRow key={k} on={f.time === k} label={l}
          onClick={() => setF(v => ({ ...v, time: k }))} />
      ))}
      <SectionTitle>项目目录</SectionTitle>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
        {dirs.map(d => {
          const on = f.dir === d;
          return (
            <button key={d} className="mono" onClick={() => setF(v => ({ ...v, dir: on ? null : d }))}
              style={{ height: 24, padding: "0 9px", borderRadius: 20,
                border: `1px solid ${on ? ACCENT : "var(--line)"}`, background: on ? "var(--acc-soft)" : "var(--surface)",
                color: on ? ACCENT : "var(--tx3)", fontSize: 11, cursor: "pointer" }}>{d}</button>
          );
        })}
        {dirs.length === 0 && <span style={{ fontSize: 11.5, color: "var(--tx5)" }}>暂无目录</span>}
      </div>
      <SectionTitle>内容</SectionTitle>
      <CheckRow on={f.mig} label="仅含迁移记录"
        onClick={() => setF(v => ({ ...v, mig: !v.mig }))} />
      <CheckRow on={f.sub} label="仅含子会话"
        onClick={() => setF(v => ({ ...v, sub: !v.sub }))} />
    </PopShell>
  );
}

// 迁移历史筛选:来源 / 目标 / 状态 / 时间
export function HistoryFilter({ f, setF, onClose, onClear }) {
  return (
    <PopShell onClose={onClose} onClear={onClear}>
      <SectionTitle first>来源工具</SectionTitle>
      {TOOLS.map(t => (
        <CheckRow key={t} on={f.src.includes(t)} icon={<ToolIcon tool={t} size={24} />}
          label={TOOL_NAME[t]}
          onClick={() => setF(v => ({ ...v, src: v.src.includes(t)
            ? v.src.filter(x => x !== t) : [...v.src, t] }))} />
      ))}
      <SectionTitle>目标工具</SectionTitle>
      {[["all", "全部目标"], ...TOOLS.map(t => [t, TOOL_NAME[t]])].map(([k, l]) => (
        <RadioRow key={k} on={f.target === k} label={l}
          onClick={() => setF(v => ({ ...v, target: k }))} />
      ))}
      <SectionTitle>状态</SectionTitle>
      {["all", "成功", "失败", "已回滚"].map(k => (
        <RadioRow key={k} on={f.status === k} label={k === "all" ? "全部状态" : k}
          onClick={() => setF(v => ({ ...v, status: k }))} />
      ))}
      <SectionTitle>时间范围</SectionTitle>
      {[["all", "全部时间"], ["today", "今天"], ["yesterday", "昨天"], ["earlier", "更早"]].map(([k, l]) => (
        <RadioRow key={k} on={f.time === k} label={l}
          onClick={() => setF(v => ({ ...v, time: k }))} />
      ))}
    </PopShell>
  );
}

// 快照筛选:来源工具 / 创建原因 / 关联会话 / 时间
export function SnapFilter({ f, setF, sessions, reasons, onClose, onClear }) {
  return (
    <PopShell onClose={onClose} onClear={onClear}>
      <SectionTitle first>来源工具</SectionTitle>
      {TOOLS.map(t => (
        <CheckRow key={t} on={f.src.includes(t)} icon={<ToolIcon tool={t} size={24} />}
          label={TOOL_NAME[t]}
          onClick={() => setF(v => ({ ...v, src: v.src.includes(t)
            ? v.src.filter(x => x !== t) : [...v.src, t] }))} />
      ))}
      <SectionTitle>创建原因</SectionTitle>
      {[["all", "全部原因"], ...reasons.map(r => [r, r])].map(([k, l]) => (
        <RadioRow key={k} on={f.reason === k} label={l}
          onClick={() => setF(v => ({ ...v, reason: k }))} />
      ))}
      <SectionTitle>关联会话</SectionTitle>
      {[["all", "全部会话"], ...sessions.map(s => [s, s])].map(([k, l]) => (
        <RadioRow key={k} on={f.session === k} label={l}
          onClick={() => setF(v => ({ ...v, session: k }))} />
      ))}
      <SectionTitle>时间范围</SectionTitle>
      {[["all", "全部时间"], ["today", "今天"], ["yesterday", "昨天"], ["earlier", "更早"]].map(([k, l]) => (
        <RadioRow key={k} on={f.time === k} label={l}
          onClick={() => setF(v => ({ ...v, time: k }))} />
      ))}
    </PopShell>
  );
}

// ---------- 快速上手引导(coach marks) ----------
const GUIDE_STEPS = [
  { target: "rail", side: "right", title: "用导航轨道切换模块",
    body: "最左侧固定轨道在会话、迁移、快照之间切换。切换时轨道位置与宽度始终不变,只更换中间资源栏与右侧详情。" },
  { target: "search", side: "right", title: "在资源栏搜索与筛选",
    body: "资源栏的标题、数量、搜索框与筛选位置在三种模块中保持一致。用它们按来源、时间与目录快速定位。" },
  { target: "scope", side: "top", scroll: true, title: "迁移到此为止并交付",
    body: "长会话可在某一轮「迁移到此为止」截断,预览迁移损耗后再交付;若验收失败会自动回滚,不留残留。" },
];
const GUIDE_TOTAL = GUIDE_STEPS.length;

export function Guide({ step, onGo, onFinish }) {
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
            borderRadius: 9, outline: `2px solid ${ACCENT}`,
            boxShadow: "0 0 0 4px var(--ring)", pointerEvents: "none",
            transition: "all .26s cubic-bezier(.2,.7,.3,1)" }} />
        </>
      )}
      <div style={{ position: "absolute", left: card?.left ?? -9999, top: card?.top ?? 0, width: 324,
        background: "var(--bg)", borderRadius: 11,
        boxShadow: "0 18px 44px -16px rgba(20,28,38,.5),0 0 0 1px var(--ring)",
        padding: "16px 18px 14px", transition: "all .26s cubic-bezier(.2,.7,.3,1)",
        animation: "fslide .16s ease" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ fontSize: 11, fontWeight: 700, color: ACCENT, letterSpacing: ".03em" }}>
            {step} / {GUIDE_TOTAL}</span>
          <div style={{ display: "flex", gap: 4, marginLeft: 2 }}>
            {GUIDE_STEPS.map((_, idx) => idx + 1).map(i => (
              <span key={i} style={{ width: 16, height: 3, borderRadius: 2,
                background: i <= step ? ACCENT : "var(--dots)" }} />))}
          </div>
          <span style={{ flex: 1 }} />
          <a onClick={onFinish} style={{ fontSize: 11.5, color: "var(--tx5)" }}>跳过</a>
        </div>
        <div style={{ fontSize: 14.5, fontWeight: 650, marginTop: 11, letterSpacing: "-.01em" }}>{cfg.title}</div>
        <div style={{ fontSize: 12.5, color: "var(--tx3)", lineHeight: 1.55, marginTop: 6 }}>{cfg.body}</div>
        <div style={{ display: "flex", alignItems: "center", gap: 9, marginTop: 15 }}>
          {step > 1 && (
            <button className="fbtn" style={{ height: 31, fontSize: 12.5 }}
              onClick={() => onGo(step - 1)}>上一步</button>)}
          <span style={{ flex: 1 }} />
          <button className="fbtn-primary" style={{ height: 31, padding: "0 16px", fontSize: 12.5 }}
            onClick={() => step >= GUIDE_TOTAL ? onFinish() : onGo(step + 1)}>
            {step >= GUIDE_TOTAL ? "开始使用" : "下一步"}</button>
        </div>
      </div>
    </div>
  );
}
