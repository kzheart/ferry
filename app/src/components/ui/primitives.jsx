// 共享 UI 构件:弹层容器 / 影响三栏 / 水位条 / 复制按钮等
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { renderEvents } from "../../api/contract/events.js";

// 居中模态(带遮罩)
export function Sheet({ width = 720, maxHeight = 800, onClose, children, z = 30 }) {
  return (
    <div style={{ position: "absolute", inset: 0, background: "var(--scrim)",
      display: "flex", alignItems: "center", justifyContent: "center", zIndex: z,
       }}
      onClick={onClose}>
      <div onClick={e => e.stopPropagation()}
        style={{ width, maxHeight, background: "var(--bg)", borderRadius: 13,
          boxShadow: "var(--shadow-sheet)", display: "flex",
          flexDirection: "column", overflow: "hidden",
           }}>
        {children}
      </div>
    </div>
  );
}

const COLS_KEY = {
  keep: { border: "var(--ok-line)", bg: "var(--ok-bg)", head: "var(--ok-deep)", dot: "var(--ok)", body: "var(--ok-body)", titleKey: "overlays:loss.keepTitle" },
  down: { border: "var(--warn-line)", bg: "var(--warn-bg)", head: "var(--warn-deep)", dot: "var(--warn)", body: "var(--warn-text)", titleKey: "overlays:loss.downTitle" },
  drop: { border: "var(--err-line)", bg: "var(--err-bg)", head: "var(--err-deep)", dot: "var(--err)", body: "var(--err-text)", titleKey: "overlays:loss.dropTitle" },
};

function LossCol({ kind, items, t }) {
  const c = COLS_KEY[kind];
  return (
    <div style={{ border: `1px solid ${c.border}`, background: c.bg, borderRadius: 10, padding: 12 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 7, fontSize: 12, fontWeight: 600, color: c.head }}>
        <span style={{ width: 7, height: 7, borderRadius: "50%", background: c.dot }} />{t(c.titleKey)}
      </div>
      {(items.length ? items : [t("overlays:loss.emptyItem")]).map((txt, i) => (
        <div key={i} style={{ fontSize: 11, color: c.body, marginTop: 7, lineHeight: 1.4 }}>{txt}</div>
      ))}
    </div>
  );
}

const clipList = (arr, max, t) => {
  const uniq = [...new Set(arr || [])];
  if (uniq.length <= max) return uniq;
  return [...uniq.slice(0, max), t("overlays:loss.moreItems", { n: uniq.length })];
};

// 迁移影响三栏(迁移预演 / 迁移历史共用)
export function LossCols({ loss }) {
  const { t } = useTranslation();
  if (!loss) return null;
  const keep = [t("overlays:loss.keepNative", { n: loss.native }), t("overlays:loss.keepRoles"), t("overlays:loss.keepRefs")];
  const down = loss.degrade ? clipList(renderEvents(loss.degrade_details), 3, t) : [];
  const drop = loss.drop ? clipList(renderEvents(loss.drop_details), 3, t) : [];
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10 }}>
      <LossCol kind="keep" items={keep} t={t} />
      <LossCol kind="down" items={down} t={t} />
      <LossCol kind="drop" items={drop} t={t} />
    </div>
  );
}

// 命令 + 复制按钮行(卡片内)
export function CmdRow({ cmd, head }) {
  const { t } = useTranslation();
  const text = typeof cmd === "string" ? cmd : cmd?.display_command || "";
  const [copied, setCopied] = useState(false);
  const copy = () => {
    try { navigator.clipboard?.writeText(text); } catch {}
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  };
  return (
    <div style={{ border: "1px solid var(--line3)", borderRadius: 10, overflow: "hidden" }}>
      {head && <div style={{ padding: "9px 13px", background: "var(--fill2)", borderBottom: "1px solid var(--line5)",
        fontSize: 11, color: "var(--tx4)", fontWeight: 600 }}>{head}</div>}
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "12px 14px" }}>
        <code className="mono selectable" style={{ flex: 1, fontSize: 12, color: "var(--tx2)",
          whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{text}</code>
        <button className="fbtn" onClick={copy}>{copied ? t("overlays:cmd.copied") : t("overlays:cmd.copy")}</button>
      </div>
    </div>
  );
}

// 复选框样子的小方块
export function CheckSquare({ on, accent = "var(--accent)", fg = "var(--accent-fg)", size = 15 }) {
  return (
    <span style={{ width: size, height: size, flex: "none", borderRadius: 4,
      border: `1.5px solid ${on ? accent : "var(--check)"}`, background: on ? accent : "transparent",
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      color: fg, fontSize: 10 }}>{on ? "✓" : ""}</span>
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
