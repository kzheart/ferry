// 标题栏索引进度胶囊:内容索引没就绪时常显(小船 + 百分比 + 剩余时间),
// 完成后变绿打勾停 3 秒再消失——不弹 toast,不抢注意力。
// 会话列表与浏览不依赖内容索引,胶囊只回答「全文搜索什么时候完整」。
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BoatGlyph, useIndexProgress } from "../modules/onboarding/public.js";

export function IndexProgressPill() {
  const { t } = useTranslation();
  const { contentIndex: ci } = useIndexProgress({ interval: 3000 });
  const [justDone, setJustDone] = useState(false);
  const wasBusy = useRef(false);

  const ready = !!ci?.ready;
  useEffect(() => {
    if (ci && !ready) { wasBusy.current = true; return undefined; }
    if (ready && wasBusy.current) {
      wasBusy.current = false;
      setJustDone(true);
      const timer = setTimeout(() => setJustDone(false), 3000);
      return () => clearTimeout(timer);
    }
    return undefined;
  }, [ci, ready]);

  if (!ci) return null;

  if (ready) {
    if (!justDone) return null;
    return (
      <span style={{ display: "inline-flex", alignItems: "center", gap: 6, height: 24,
        padding: "0 11px", borderRadius: 99, border: "1px solid var(--ok)",
        fontSize: 11, fontWeight: 600, color: "var(--ok-deep)", flex: "none" }}>
        ✓ {t("app:indexPill.done")}
      </span>
    );
  }

  const total = (ci.indexed_sessions || 0) + (ci.pending_sessions || 0);
  if (!total) return null;
  const pct = Math.min(100, Math.round((ci.indexed_sessions || 0) / total * 100));

  return (
    <span title={t("app:indexPill.tip", { indexed: ci.indexed_sessions, total })}
      style={{ display: "inline-flex", alignItems: "center", gap: 8, height: 24,
        padding: "0 11px", borderRadius: 99, border: "1px solid var(--line3)",
        fontSize: 11, fontWeight: 600, color: "var(--tx3b)", flex: "none",
        fontVariantNumeric: "tabular-nums" }}>
      <span className="fw-boat" style={{ color: "var(--accent)", display: "flex",
        position: "static" }}>
        <BoatGlyph />
      </span>
      {t("app:indexPill.busy")} {pct}%
      <span style={{ width: 52, height: 3, borderRadius: 99, background: "var(--fill4)",
        overflow: "hidden" }}>
        <i style={{ display: "block", height: "100%", borderRadius: 99, width: `${pct}%`,
          background: "var(--accent)", transition: "width .5s linear" }} />
      </span>
    </span>
  );
}
