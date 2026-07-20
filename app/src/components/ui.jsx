// 共享 UI 构件:弹层容器 / 损耗三栏 / 水位条 / 复制按钮等
import { useState } from "react";

// 居中模态(带遮罩)
export function Sheet({ width = 720, maxHeight = 800, onClose, children, z = 30 }) {
  return (
    <div style={{ position: "absolute", inset: 0, background: "var(--scrim)",
      display: "flex", alignItems: "center", justifyContent: "center", zIndex: z,
      animation: "ffade .18s ease" }}
      onClick={onClose}>
      <div onClick={e => e.stopPropagation()}
        style={{ width, maxHeight, background: "var(--bg)", borderRadius: 13,
          boxShadow: "0 30px 70px -20px rgba(20,28,38,.5)", display: "flex",
          flexDirection: "column", overflow: "hidden",
          animation: "fsheet .22s cubic-bezier(.2,.7,.3,1)" }}>
        {children}
      </div>
    </div>
  );
}

const COLS = {
  keep: { border: "var(--ok-line)", bg: "var(--ok-bg)", head: "var(--ok-deep)", dot: "var(--ok)", body: "var(--ok-body)", title: "原生保留" },
  down: { border: "var(--warn-line)", bg: "var(--warn-bg)", head: "var(--warn-deep)", dot: "var(--warn)", body: "var(--warn-text)", title: "降级转换" },
  drop: { border: "var(--err-line)", bg: "var(--err-bg)", head: "var(--err-deep)", dot: "var(--err)", body: "var(--err-text)", title: "无法迁移" },
};

function LossCol({ kind, items }) {
  const c = COLS[kind];
  return (
    <div style={{ border: `1px solid ${c.border}`, background: c.bg, borderRadius: 10, padding: 12 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 7, fontSize: 12, fontWeight: 600, color: c.head }}>
        <span style={{ width: 7, height: 7, borderRadius: "50%", background: c.dot }} />{c.title}
      </div>
      {(items.length ? items : ["无"]).map((t, i) => (
        <div key={i} style={{ fontSize: 11.5, color: c.body, marginTop: 7, lineHeight: 1.4 }}>{t}</div>
      ))}
    </div>
  );
}

const clipList = (arr, max = 3) => {
  const uniq = [...new Set(arr || [])];
  if (uniq.length <= max) return uniq;
  return [...uniq.slice(0, max), `… 等共 ${uniq.length} 项`];
};

// 损耗报告三栏(迁移预演 / 迁移历史共用)
export function LossCols({ loss }) {
  if (!loss) return null;
  const keep = [`${loss.native} 个内容块原生映射`, "消息角色与顺序", "文件引用与代码块"];
  const down = loss.degrade ? clipList(loss.degrade_details) : [];
  const drop = loss.drop ? clipList(loss.drop_details) : [];
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10 }}>
      <LossCol kind="keep" items={keep} />
      <LossCol kind="down" items={down} />
      <LossCol kind="drop" items={drop} />
    </div>
  );
}

// 命令 + 复制按钮行(卡片内)
export function CmdRow({ cmd, head }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    try { navigator.clipboard?.writeText(cmd); } catch {}
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  };
  return (
    <div style={{ border: "1px solid var(--line3)", borderRadius: 10, overflow: "hidden" }}>
      {head && <div style={{ padding: "9px 13px", background: "var(--fill2)", borderBottom: "1px solid var(--line5)",
        fontSize: 11.5, color: "var(--tx4)", fontWeight: 600 }}>{head}</div>}
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "12px 14px" }}>
        <code className="mono selectable" style={{ flex: 1, fontSize: 12.5, color: "var(--tx2)",
          whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{cmd}</code>
        <button className="fbtn" onClick={copy}>{copied ? "已复制" : "复制"}</button>
      </div>
    </div>
  );
}

// 复选框样子的小方块
export function CheckSquare({ on, accent = "var(--accent)", size = 15 }) {
  return (
    <span style={{ width: size, height: size, flex: "none", borderRadius: 4,
      border: `1.5px solid ${on ? accent : "var(--check)"}`, background: on ? accent : "transparent",
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      color: "#fff", fontSize: 10 }}>{on ? "✓" : ""}</span>
  );
}

// 单选圆点
export function RadioDot({ on, accent = "var(--accent)", size = 15 }) {
  return (
    <span style={{ width: size, height: size, flex: "none", borderRadius: "50%",
      border: `1.5px solid ${on ? accent : "var(--check)"}`, display: "inline-flex",
      alignItems: "center", justifyContent: "center" }}>
      <span style={{ width: size * 0.47, height: size * 0.47, borderRadius: "50%",
        background: on ? accent : "transparent" }} />
    </span>
  );
}

export function StatusPill({ label, color, bg }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 6, padding: "4px 11px",
      borderRadius: 20, fontSize: 12, fontWeight: 600, background: bg, color, flex: "none" }}>
      <span style={{ width: 6, height: 6, borderRadius: "50%", background: color }} />{label}
    </span>
  );
}
