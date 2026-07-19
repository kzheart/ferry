// 其余弹层:差异预览 / 原地修改确认 / 快照还原确认 / 结果 toast /
// 设置弹层 / 数据来源 sheet / 三个筛选弹层 / 快速上手引导
import { useEffect, useState } from "react";
import { ACCENT, TOOL_NAME, TOOLS, fmtSize, fmtTime } from "../api.js";
import { PlusIcon, Spinner, ToolIcon } from "../icons.jsx";
import { CheckSquare, RadioDot, Sheet } from "../components/ui.jsx";

// ---------- 差异预览 ----------
export function DiffSheet({ ops, preview, loading, onClose }) {
  return (
    <Sheet width={760} maxHeight={780} onClose={onClose}>
      <div style={{ flex: "none", padding: "15px 20px", borderBottom: "1px solid #E8ECF0",
        display: "flex", alignItems: "center" }}>
        <div style={{ fontSize: 14.5, fontWeight: 650 }}>差异预览</div>
        <div style={{ fontSize: 12, color: "#8A939D", marginLeft: 12 }}>
          {ops.length} 项暂存操作
          {preview && ` · ${fmtSize(preview.before.size)} → ${fmtSize(preview.after.size)}
            · ${preview.before.count} → ${preview.after.count} 条记录`}
        </div>
        <div style={{ flex: 1 }} />
        <a onClick={onClose} style={{ color: "#9AA3AD", fontSize: 18 }}>×</a>
      </div>
      <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "18px 20px" }}>
        {ops.length === 0 && (
          <div style={{ textAlign: "center", color: "#9AA3AD", fontSize: 13, padding: 40 }}>尚无暂存操作</div>)}
        {loading && (
          <div style={{ display: "flex", alignItems: "center", gap: 10, color: "#8A939D",
            fontSize: 12.5, marginBottom: 14 }}><Spinner size={14} /> 正在计算前后差异…</div>)}
        {ops.map(o => (
          <div key={o.id} style={{ border: "1px solid #E4E9EE", borderRadius: 10, overflow: "hidden",
            marginBottom: 12 }}>
            <div style={{ padding: "9px 13px", background: "#F4F7F9", borderBottom: "1px solid #E8ECF0",
              display: "flex", alignItems: "center", gap: 9 }}>
              <span style={{ width: 7, height: 7, borderRadius: "50%", background: o.dot }} />
              <span style={{ fontSize: 12.5, fontWeight: 600, color: "#334155" }}>{o.label}</span>
              <span style={{ fontSize: 11, color: "#9AA3AD", marginLeft: "auto" }}>{o.delta}</span>
            </div>
            <div className="mono" style={{ padding: "11px 13px", fontSize: 11.5, lineHeight: 1.7 }}>
              <div style={{ background: "#FBEDEC", color: "#9A3E37", padding: "2px 8px", borderRadius: 5,
                marginBottom: 4, textDecoration: o.type === "delete" ? "line-through" : "none",
                whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>− {o.before}</div>
              {o.after && (
                <div style={{ background: "#EAF6EE", color: "#2A6B44", padding: "2px 8px", borderRadius: 5,
                  whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>+ {o.after}</div>)}
            </div>
          </div>
        ))}
        {preview?.notes?.length > 0 && (
          <div style={{ fontSize: 12, color: "#6B7682", lineHeight: 1.6 }}>
            引擎确认:{preview.notes.join(";")}</div>)}
      </div>
      <div style={{ flex: "none", padding: "13px 20px", borderTop: "1px solid #E8ECF0",
        display: "flex", justifyContent: "flex-end" }}>
        <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onClose}>关闭</button>
      </div>
    </Sheet>
  );
}

// ---------- 小确认框 ----------
function ConfirmBox({ width = 400, title, children, actions }) {
  return (
    <div style={{ position: "absolute", inset: 0, background: "rgba(24,33,43,.34)", display: "flex",
      alignItems: "center", justifyContent: "center", zIndex: 44, animation: "ffade .15s ease" }}>
      <div style={{ width, background: "#FBFCFD", borderRadius: 12,
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
      <button style={{ height: 34, padding: "0 16px", background: "#C4564C", border: "none",
        borderRadius: 8, fontSize: 13, color: "#fff", cursor: "pointer", fontWeight: 600 }}
        onClick={onConfirm}>确认原地修改</button>
    </>}>
      <div style={{ fontSize: 12.5, color: "#6B7682", marginTop: 8, lineHeight: 1.55 }}>
        这会直接改写原始会话文件。Ferry 会先自动创建快照;若应用后探针失败将自动还原。此操作可通过快照撤销。</div>
    </ConfirmBox>
  );
}

export function SnapRestoreConfirm({ snap, onCancel, onConfirm }) {
  const bullets = [
    ["#E09112", "当前会话在此快照之后的改动将被覆盖。"],
    ["#1C9E5A", "Ferry 会在还原前自动创建一个当前状态的保护快照。"],
    ["#0B67F5", "还原完成后可通过该保护快照撤销本次操作。"],
    ["#8AA0B6", "源工具中的其他会话不受影响。"],
  ];
  return (
    <ConfirmBox width={440} title="还原到此快照?" actions={<>
      <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>取消</button>
      <button className="fbtn-primary" style={{ height: 34, padding: "0 16px", fontSize: 13 }}
        onClick={onConfirm}>创建保护快照并还原</button>
    </>}>
      <div style={{ fontSize: 12.5, color: "#6B7682", marginTop: 7, lineHeight: 1.5 }}>
        会话「{snap.title}」将恢复到 {fmtTime(snap.time)} 的状态。</div>
      <div style={{ marginTop: 14, border: "1px solid #E4E9EE", borderRadius: 10, padding: "12px 14px",
        display: "flex", flexDirection: "column", gap: 9 }}>
        {bullets.map(([c, t], i) => (
          <div key={i} style={{ display: "flex", gap: 9, fontSize: 12, color: "#40494F", lineHeight: 1.45 }}>
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
  const bg = kind === "fail" ? "#FDF3F1" : kind === "ok" ? "#F1FBF5" : "#FBFCFD";
  const border = kind === "fail" ? "#EBCBC7" : kind === "ok" ? "#CDE9D7" : "#E4E9EE";
  const color = kind === "fail" ? "#8A3E37" : kind === "ok" ? "#1C7C43" : "#334155";
  return (
    <div style={{ position: "absolute", left: "50%", bottom: 26, transform: "translateX(-50%)",
      zIndex: 45, display: "flex", alignItems: "center", gap: 11, padding: "12px 16px",
      borderRadius: 10, background: bg, border: `1px solid ${border}`,
      boxShadow: "0 12px 30px -12px rgba(20,28,38,.4)", animation: "fsheet .22s ease", maxWidth: 560 }}>
      {kind === "run" ? <Spinner size={20} track="#E1E7EC" /> : (
        <span style={{ width: 26, height: 26, flex: "none", borderRadius: "50%",
          background: kind === "ok" ? "#1C9E5A" : "#D5544A", display: "inline-flex",
          alignItems: "center", justifyContent: "center", color: "#fff", fontSize: 14 }}>
          {kind === "ok" ? "✓" : "×"}</span>)}
      <div>
        <div style={{ fontSize: 13, fontWeight: 600, color }}>{toast.title}</div>
        <div style={{ fontSize: 11.5, color: "#6B7682", marginTop: 2 }}>{toast.desc}</div>
      </div>
      <a onClick={onDismiss} style={{ color: "#9AA3AD", fontSize: 16, marginLeft: 6 }}>×</a>
    </div>
  );
}

// ---------- 设置弹层 ----------
export function SettingsPopover({ onClose, onOpenGuide, onFirstRun, guideSeen }) {
  return (
    <>
      <div onClick={onClose} style={{ position: "absolute", inset: 0, zIndex: 45 }} />
      <div style={{ position: "absolute", left: 62, bottom: 14, width: 252, zIndex: 46,
        background: "#FBFCFD", borderRadius: 11,
        boxShadow: "0 16px 40px -14px rgba(20,28,38,.42),0 0 0 1px rgba(20,28,38,.08)",
        padding: "13px 14px", animation: "fpop .14s ease" }}>
        <div style={{ fontSize: 13, fontWeight: 650, marginBottom: 10 }}>设置</div>
        <button className="hov-item" onClick={onOpenGuide}
          style={{ width: "100%", height: 32, textAlign: "left", padding: "0 10px",
            background: "transparent", border: "none", borderRadius: 7, fontSize: 12.5,
            color: "#334155", cursor: "pointer" }}>
          {guideSeen ? "重新查看引导" : "快速上手"}</button>
        <button className="hov-item" onClick={onFirstRun}
          style={{ width: "100%", height: 32, textAlign: "left", padding: "0 10px",
            background: "transparent", border: "none", borderRadius: 7, fontSize: 12.5,
            color: "#334155", cursor: "pointer" }}>首次启动检测</button>
        <div style={{ fontSize: 11, color: "#9AA3AD", marginTop: 8, lineHeight: 1.5 }}>
          Ferry 在本机运行,不上传任何数据;已适配「减少动效」。</div>
      </div>
    </>
  );
}

// ---------- 数据来源 sheet ----------
export function DataSourceSheet({ scan, env, scanning, onRescan, onClose, onOpenGuide, onFirstRun, guideSeen }) {
  const tools = scan?.tools || {};
  const connected = TOOLS.filter(t => tools[t]?.ok).length;
  const total = TOOLS.reduce((a, t) => a + (tools[t]?.count || 0), 0);
  return (
    <Sheet width={540} maxHeight={660} onClose={onClose} z={38}>
      <div style={{ flex: "none", padding: "15px 20px", borderBottom: "1px solid #E8ECF0",
        display: "flex", alignItems: "center" }}>
        <div>
          <div style={{ fontSize: 14.5, fontWeight: 650 }}>数据来源</div>
          <div style={{ fontSize: 11.5, color: "#8A939D", marginTop: 2 }}>
            {connected} 个已连接 · {total} 个会话 · 源会话保持只读</div>
        </div>
        <div style={{ flex: 1 }} />
        <a onClick={onClose} style={{ color: "#9AA3AD", fontSize: 18 }}>×</a>
      </div>
      <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "12px 16px" }}>
        {TOOLS.map(t => {
          const info = tools[t] || {};
          const ok = info.ok;
          return (
            <div key={t} style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 13px",
              border: "1px solid #E4E9EE", borderRadius: 10, marginBottom: 9, background: "#fff" }}>
              <ToolIcon tool={t} size={30} dot={ok ? "#1C9E5A" : "#D5544A"} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: "#334155" }}>{TOOL_NAME[t]}
                  {env?.[t]?.version && <span style={{ fontWeight: 400, color: "#9AA3AD",
                    fontSize: 11.5 }}> · v{env[t].version}</span>}</div>
                <div className="mono" style={{ fontSize: 11, color: "#9AA3AD", whiteSpace: "nowrap",
                  overflow: "hidden", textOverflow: "ellipsis" }}>{info.path || "—"}</div>
              </div>
              <div style={{ textAlign: "right", flex: "none" }}>
                <div style={{ fontSize: 11.5, color: "#6B7682" }}>
                  {ok ? `${info.count} 个会话` : (info.error || "不可用")}</div>
                <div style={{ display: "inline-flex", alignItems: "center", gap: 5, fontSize: 11,
                  color: ok ? "#1C7C43" : "#B4433A", fontWeight: 600, marginTop: 2 }}>
                  <span style={{ width: 6, height: 6, borderRadius: "50%",
                    background: ok ? "#1C9E5A" : "#D5544A" }} />{ok ? "已连接" : "扫描失败"}</div>
              </div>
              <button className="fbtn" style={{ fontSize: 11.5, flex: "none" }}
                onClick={onRescan} disabled={scanning}>
                {scanning ? "扫描中…" : "重新扫描"}</button>
            </div>
          );
        })}
        <button className="hov-ghost" style={{ width: "100%", height: 40, border: "1px dashed #C3CBD3",
          background: "transparent", borderRadius: 10, fontSize: 12.5, color: "#6B7682",
          cursor: "default", display: "flex", alignItems: "center", justifyContent: "center", gap: 8 }}>
          <PlusIcon />添加数据来源(规划中)
        </button>
        <div style={{ fontSize: 11, color: "#9AA3AD", marginTop: 8, lineHeight: 1.5 }}>
          未来新增本地工具时在此接入,主界面不会增加固定标签。</div>
      </div>
      <div style={{ flex: "none", padding: "12px 16px", borderTop: "1px solid #E8ECF0",
        display: "flex", alignItems: "center", gap: 9 }}>
        <button className="fbtn" style={{ height: 32, fontSize: 12.5 }} onClick={onOpenGuide}>
          {guideSeen ? "重新查看引导" : "快速上手"}</button>
        <button className="fbtn" style={{ height: 32, fontSize: 12.5 }} onClick={onFirstRun}>首次启动检测</button>
        <span style={{ flex: 1 }} />
        <button className="fbtn-primary" style={{ height: 32, padding: "0 16px" }} onClick={onClose}>完成</button>
      </div>
    </Sheet>
  );
}

// ---------- 筛选弹层(共用外壳) ----------
function PopShell({ onClose, onClear, children }) {
  return (
    <>
      <div onClick={onClose} style={{ position: "absolute", inset: 0, zIndex: 35 }} />
      <div style={{ position: "absolute", left: 66, top: 190, width: 272, zIndex: 36,
        background: "#FBFCFD", borderRadius: 11,
        boxShadow: "0 16px 40px -14px rgba(20,28,38,.42),0 0 0 1px rgba(20,28,38,.08)",
        overflow: "hidden", animation: "fpop .14s ease" }}>
        <div className="fscroll" style={{ maxHeight: 430, overflowY: "auto", padding: "12px 13px" }}>
          {children}
        </div>
        <div style={{ display: "flex", alignItems: "center", padding: "9px 13px",
          borderTop: "1px solid #E8ECF0" }}>
          <a onClick={onClear} style={{ fontSize: 11.5, color: "#6B7682" }}>清除筛选</a>
          <span style={{ flex: 1 }} />
          <button className="fbtn-primary" style={{ height: 28, padding: "0 14px", fontSize: 12 }}
            onClick={onClose}>完成</button>
        </div>
      </div>
    </>
  );
}

const SectionTitle = ({ children, first }) => (
  <div style={{ fontSize: 11, fontWeight: 600, color: "#9AA3AD", letterSpacing: ".03em",
    margin: first ? "0 0 6px" : "12px 0 6px" }}>{children}</div>
);

function CheckRow({ on, onClick, icon, label, extra }) {
  return (
    <div className="hov-item" onClick={onClick}
      style={{ display: "flex", alignItems: "center", gap: 9, padding: "6px 7px", borderRadius: 7,
        cursor: "pointer" }}>
      <CheckSquare on={on} />
      {icon}
      <span style={{ fontSize: 12.5, color: "#334155", flex: 1, whiteSpace: "nowrap",
        overflow: "hidden", textOverflow: "ellipsis" }}>{label}</span>
      {extra && <span style={{ fontSize: 11, color: "#9AA3AD" }}>{extra}</span>}
    </div>
  );
}

function RadioRow({ on, onClick, label }) {
  return (
    <div className="hov-item" onClick={onClick}
      style={{ display: "flex", alignItems: "center", gap: 9, padding: "6px 7px", borderRadius: 7,
        cursor: "pointer" }}>
      <RadioDot on={on} />
      <span style={{ fontSize: 12.5, color: "#334155", whiteSpace: "nowrap", overflow: "hidden",
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
                border: `1px solid ${on ? ACCENT : "#E1E7EC"}`, background: on ? "#EAF0FB" : "#fff",
                color: on ? ACCENT : "#5B6672", fontSize: 11, cursor: "pointer" }}>{d}</button>
          );
        })}
        {dirs.length === 0 && <span style={{ fontSize: 11.5, color: "#9AA3AD" }}>暂无目录</span>}
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

// 快照筛选:关联会话 / 时间
export function SnapFilter({ f, setF, sessions, onClose, onClear }) {
  return (
    <PopShell onClose={onClose} onClear={onClear}>
      <SectionTitle first>关联会话</SectionTitle>
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
  { target: "scan", side: "right", title: "先扫描本机数据来源",
    body: "左下角「数据来源」里查看已连接的 Claude Code、Codex CLI、OpenCode 及已扫描会话数。源会话保持只读,可随时重新扫描。" },
  { target: "rail", side: "right", title: "用导航轨道切换模块",
    body: "最左侧固定轨道在会话、迁移、快照之间切换。切换时轨道位置与宽度始终不变,只更换中间资源栏与右侧详情。" },
  { target: "search", side: "right", title: "在资源栏搜索与筛选",
    body: "资源栏的标题、数量、搜索框与筛选位置在三种模块中保持一致。用它们按来源、时间与目录快速定位。" },
  { target: "migrate", side: "bottom", title: "迁移并交付",
    body: "在会话详情里可整体迁移,或在某一轮「迁移到此为止」截断;预演损耗与上下文水位后再交付,探针失败会自动回滚,不留残留。" },
];

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
    const t = setTimeout(run, 30);
    return () => clearTimeout(t);
  }, [step]);

  if (!cfg) return null;
  const b = box || { l: -9999, t: 0, w: 0, h: 0, W: 4000, H: 3000 };
  const dim = "rgba(20,28,38,.44)";
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
            boxShadow: "0 0 0 4px rgba(11,103,245,.2)", pointerEvents: "none",
            transition: "all .26s cubic-bezier(.2,.7,.3,1)" }} />
        </>
      )}
      <div style={{ position: "absolute", left: card?.left ?? -9999, top: card?.top ?? 0, width: 324,
        background: "#FBFCFD", borderRadius: 11,
        boxShadow: "0 18px 44px -16px rgba(20,28,38,.5),0 0 0 1px rgba(20,28,38,.08)",
        padding: "16px 18px 14px", transition: "all .26s cubic-bezier(.2,.7,.3,1)",
        animation: "fslide .16s ease" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ fontSize: 11, fontWeight: 700, color: ACCENT, letterSpacing: ".03em" }}>
            {step} / 4</span>
          <div style={{ display: "flex", gap: 4, marginLeft: 2 }}>
            {[1, 2, 3, 4].map(i => (
              <span key={i} style={{ width: 16, height: 3, borderRadius: 2,
                background: i <= step ? ACCENT : "#D8DEE4" }} />))}
          </div>
          <span style={{ flex: 1 }} />
          <a onClick={onFinish} style={{ fontSize: 11.5, color: "#9AA3AD" }}>跳过</a>
        </div>
        <div style={{ fontSize: 14.5, fontWeight: 650, marginTop: 11, letterSpacing: "-.01em" }}>{cfg.title}</div>
        <div style={{ fontSize: 12.5, color: "#5B6672", lineHeight: 1.55, marginTop: 6 }}>{cfg.body}</div>
        <div style={{ display: "flex", alignItems: "center", gap: 9, marginTop: 15 }}>
          {step > 1 && (
            <button className="fbtn" style={{ height: 31, fontSize: 12.5 }}
              onClick={() => onGo(step - 1)}>上一步</button>)}
          <span style={{ flex: 1 }} />
          <button className="fbtn-primary" style={{ height: 31, padding: "0 16px", fontSize: 12.5 }}
            onClick={() => step >= 4 ? onFinish() : onGo(step + 1)}>
            {step >= 4 ? "开始使用" : "下一步"}</button>
        </div>
      </div>
    </div>
  );
}
