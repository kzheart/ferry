// 首次启动的索引进度胶囊:内容索引没就绪时常显(小船 + 百分比),
// 完成后变绿打勾停 3 秒再永久退场。后续索引完全静默。
// 会话列表与浏览不依赖内容索引,胶囊只回答首次启动时「全文搜索什么时候完整」。
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BoatGlyph, useIndexProgress } from "../modules/onboarding/public.js";

/** 完成印记:虚线航线收拢成圆环,对号靠港。动效见 app.css 的 .fw-seal。 */
function SealGlyph() {
  return (
    <svg className="fw-seal" width="14" height="14" viewBox="0 0 22 22" aria-hidden>
      <circle cx="11" cy="11" r="9" />
      <path d="M6.8 11.4 9.8 14.4 15.2 8.4" />
    </svg>
  );
}

export function IndexProgressPill({ active = false }) {
  const { t } = useTranslation();
  const [phase, setPhase] = useState("tracking");
  const { contentIndex: ci } = useIndexProgress({
    active: active && phase === "tracking",
    interval: 3000,
  });
  const wasBusy = useRef(false);

  const ready = !!ci?.ready;
  const hasProgress = ci != null;

  // 只对首次索引的 busy -> ready 边沿展示完成态。若挂载时已经 ready，说明用户
  // 在向导里等完了，直接退场即可；切到 done 后也立刻停止轮询，后续增量索引静默。
  useEffect(() => {
    if (!active || phase !== "tracking" || !hasProgress) return;
    if (!ready) {
      wasBusy.current = true;
      return;
    }
    if (wasBusy.current) {
      wasBusy.current = false;
      setPhase("done");
    } else {
      setPhase("retired");
    }
  }, [active, hasProgress, phase, ready]);

  useEffect(() => {
    if (phase !== "done") return undefined;
    const timer = setTimeout(() => setPhase("retired"), 3000);
    return () => clearTimeout(timer);
  }, [phase]);

  if (!active || phase === "retired") return null;

  if (phase === "done") {
    return (
      <span style={{ display: "inline-flex", alignItems: "center", gap: 6, height: 24,
        padding: "0 11px", borderRadius: 99, border: "1px solid var(--ok)",
        fontSize: 11, fontWeight: 600, color: "var(--ok-deep)", flex: "none" }}>
        <SealGlyph /> {t("app:indexPill.done")}
      </span>
    );
  }

  if (!ci || ready) return null;

  const total = (ci.indexed_sessions || 0) + (ci.pending_sessions || 0);
  if (!total) return null;
  // 未就绪时封顶 99:「100%」只留给真正完成——尾巴上还剩几个会话时
  // 四舍五入到 100 会让胶囊看起来像卡死。
  const pct = Math.min(99, Math.floor((ci.indexed_sessions || 0) / total * 100));

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
