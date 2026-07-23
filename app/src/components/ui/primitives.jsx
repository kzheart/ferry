// 共享 UI 构件:弹层容器 / 影响三栏 / 水位条 / 复制按钮等
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { renderEvents } from "../../api/contract/events.js";
import { writeClipboardText } from "../../api/transport/rpc.js";

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
  keep: { dot: "var(--ok)", titleKey: "overlays:loss.keepTitle" },
  down: { dot: "var(--warn)", titleKey: "overlays:loss.downTitle" },
  drop: { dot: "var(--err)", titleKey: "overlays:loss.dropTitle" },
};

const clipList = (arr, max, t) => {
  const uniq = [...new Set(arr || [])];
  if (uniq.length <= max) return uniq;
  return [...uniq.slice(0, max), t("overlays:loss.moreItems", { n: uniq.length })];
};

function Dot({ color, size = 6 }) {
  return <span style={{ width: size, height: size, flex: "none", borderRadius: "50%", background: color }} />;
}

// 影响分组:标题行(名称 + 计数) + 明细行
function LossGroup({ kind, count, items, t }) {
  const c = COLS_KEY[kind];
  return (
    <div style={{ padding: "11px 14px 12px", borderTop: "1px solid var(--line5)" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
        <Dot color={c.dot} />
        <span style={{ color: "var(--tx2)", fontWeight: 600 }}>{t(c.titleKey)}</span>
        {count != null && (
          <span className="mono" style={{ marginLeft: "auto", color: "var(--tx4)" }}>{count}</span>
        )}
      </div>
      {(items.length ? items : [t("overlays:loss.emptyItem")]).map((txt, i) => (
        <div key={i} style={{ fontSize: 11.5, color: "var(--tx3b)", lineHeight: 1.45,
          marginTop: 6, paddingLeft: 14 }}>{txt}</div>
      ))}
    </div>
  );
}

// 迁移影响卡(迁移预演 / 迁移历史共用):占比条 + 图例 + 分组明细
// compact:聊天流里的缩略形态,只留占比条 + 图例,分组明细留到差异视图看
export function LossCols({ loss, compact = false }) {
  const { t } = useTranslation();
  if (!loss) return null;
  const n = { keep: loss.native || 0, down: loss.degrade || 0, drop: loss.drop || 0 };
  const total = n.keep + n.down + n.drop;
  const segs = ["keep", "down", "drop"].filter(k => n[k] > 0);
  const down = n.down ? clipList(renderEvents(loss.degrade_details), 4, t) : [];
  const drop = n.drop ? clipList(renderEvents(loss.drop_details), 4, t) : [];

  return (
    <div className={compact ? undefined : "fcard"}>
      <div style={{ padding: compact ? "10px 12px" : "13px 14px 12px" }}>
        {/* 占比条:量级差两个数量级时也要看得见,故给最小宽度 */}
        <div style={{ display: "flex", gap: 2, height: 6, borderRadius: 3, overflow: "hidden",
          background: "var(--fill4)" }}>
          {segs.map(k => (
            <div key={k} style={{ flex: n[k], minWidth: 6, borderRadius: 3, background: COLS_KEY[k].dot }} />
          ))}
        </div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: "6px 16px", marginTop: 11, fontSize: 12 }}>
          {["keep", "down", "drop"].map(k => (
            <span key={k} style={{ display: "inline-flex", alignItems: "center", gap: 7 }}>
              <Dot color={COLS_KEY[k].dot} />
              <span style={{ color: "var(--tx3b)" }}>{t(COLS_KEY[k].titleKey)}</span>
              <span className="mono" style={{ color: "var(--tx2)", fontWeight: 600 }}>{n[k]}</span>
            </span>
          ))}
          <span style={{ marginLeft: "auto", color: "var(--tx4)" }}>
            {t("overlays:loss.totalBlocks", { n: total })}</span>
        </div>
      </div>
      {!compact && (
        <LossGroup kind="keep" items={[t("overlays:loss.keepRoles"), t("overlays:loss.keepRefs")]} t={t} />)}
      {!compact && n.down > 0 && <LossGroup kind="down" count={n.down} items={down} t={t} />}
      {!compact && n.drop > 0 && <LossGroup kind="drop" count={n.drop} items={drop} t={t} />}
    </div>
  );
}

// 命令 + 复制按钮行(卡片内)
export function CmdRow({ cmd, head }) {
  const { t } = useTranslation();
  const text = typeof cmd === "string" ? cmd : cmd?.display_command || "";
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await writeClipboardText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {}
  };
  return (
    <div className="fcard" style={{ overflow: "hidden" }}>
      {head && <div style={{ padding: "9px 14px", borderBottom: "1px solid var(--line5)",
        fontSize: 11, color: "var(--tx4)", fontWeight: 600 }}>{head}</div>}
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "11px 14px" }}>
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
