// 迁移历史详情:范围/迁移影响/上下文水位/验收结果/回滚信息/接续命令
import { useTranslation } from "react-i18next";
import { probeText } from "../../api/contract/events.js";
import { TOOL_NAME } from "../../api/contract/tools.js";
import { fmtSize } from "../../domain/tools/toolDisplay.js";
import { fmtTime } from "../../domain/sessions/sessionModel.js";
import { histStatus, STATUS_CODE } from "./migrationModel.js";
import { ToolIcon, TrashIcon } from "../../components/ui/icons.jsx";
import { CmdRow, LossCols, StatusPill } from "../../components/ui/primitives.jsx";

const ST_STYLE = {
  [STATUS_CODE.success]: ["var(--ok-bg)", "var(--ok)"],
  [STATUS_CODE.failed]: ["var(--err-bg2)", "var(--err)"],
  [STATUS_CODE.rolledBack]: ["var(--chip)", "var(--tx3b)"],
  [STATUS_CODE.dryRun]: ["var(--warn-bg)", "var(--warn)"],
};

// 卡片外的小标题(右侧可挂一段次要信息)
function SectionHead({ children, aside }) {
  return (
    <div style={{ display: "flex", alignItems: "baseline", gap: 12, margin: "20px 2px 8px" }}>
      <div style={{ fontSize: 12, fontWeight: 600, color: "var(--tx3b)" }}>{children}</div>
      {aside && <div className="mono" style={{ marginLeft: "auto", fontSize: 11, color: "var(--tx5)" }}>{aside}</div>}
    </div>
  );
}

export default function HistoryDetail({ h, onDelete }) {
  const { t } = useTranslation();
  if (!h) return (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
      color: "var(--tx5)", fontSize: 13 }}>{t("migration:history.empty")}</div>
  );
  const status = histStatus(h);
  const [stBg, stColor] = ST_STYLE[status] || ST_STYLE[STATUS_CODE.failed];
  const ok = status === STATUS_CODE.success;
  const fail = status === STATUS_CODE.failed;
  const rolled = h.rolled_back;
  const range = h.max_turn ? t("migration:history.rangeToTurn", { n: h.max_turn }) : t("migration:history.rangeFull");
  const probeDetail = probeText(h.probe);
  const probeLines = h.probe
    ? (ok ? probeDetail.split("\n").filter(Boolean).slice(0, 4)
      : probeDetail.split("\n").filter(Boolean))
    : h.validation?.structure
      ? [h.validation.structure.detail, t("migration:history.probeNotRunDefault")]
      : [t("migration:history.probeNotRun")];
  // 容器保持中性,状态只由圆点承载
  const probeDot = ok ? "var(--ok)" : fail ? "var(--err)" : "var(--tx5)";
  const probeModel = h.probe?.model || h.probe_model;

  return (
    <div className="fscroll" style={{ flex: 1, overflowY: "auto", minHeight: 0 }}>
      <div style={{ padding: "20px 26px 16px", borderBottom: "1px solid var(--line5)" }}>
        <div style={{ display: "flex", alignItems: "flex-start", gap: 13, maxWidth: 720, margin: "0 auto" }}>
          <ToolIcon tool={h.src} size={40} dot={stColor} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 16, fontWeight: 650, letterSpacing: "-.01em" }}>{h.title || h.source_id}</div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "6px 14px", marginTop: 6,
              fontSize: 12, color: "var(--tx3b)" }}>
              <span className="mono" style={{ color: "var(--tx4)" }}>{h.session_id || h.source_id}</span>
              <span>{fmtTime(h.time, t)}</span>
              <span>{TOOL_NAME[h.src]} → {TOOL_NAME[h.dst]}</span>
            </div>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 8, flex: "none" }}>
            <StatusPill label={t(`common:${status}`)} color={stColor} bg={stBg} />
            {onDelete && h.id && (
              <button className="ftool-btn" title={t("migration:history.delete")}
                onClick={onDelete}>
                <TrashIcon size={14} />
              </button>)}
          </div>
        </div>
      </div>
      <div style={{ padding: "20px 26px 44px", maxWidth: 720, margin: "0 auto" }}>
        {/* macOS 设置式分组卡片:面板底 + 行间发丝线 */}
        <div className="fcard" style={{ fontSize: 12 }}>
          {[
            [t("migration:history.fieldRange"),
              h.msg_count ? t("migration:history.rangeWithCount", { range, n: h.msg_count }) : range],
            [t("migration:history.fieldSrcToDst"), `${TOOL_NAME[h.src]} → ${TOOL_NAME[h.dst]}`],
            ...(h.tree_count != null
              ? [[t("migration:history.fieldTree"),
                  t("migration:history.treeMeta", { n: h.tree_count, detail: h.topology?.detail ? ` · ${h.topology.detail}` : "" })]]
              : []),
          ].map(([k, v], i) => (
            <div key={k} style={{ display: "flex", justifyContent: "space-between", gap: 16,
              padding: "10px 14px", borderTop: i ? "1px solid var(--line5)" : "none" }}>
              <span style={{ color: "var(--tx4)", flex: "none" }}>{k}</span>
              <span style={{ color: "var(--tx2)", textAlign: "right" }}>{v}</span>
            </div>
          ))}
        </div>

        <SectionHead>{t("migration:history.impactReport")}</SectionHead>
        <LossCols loss={h.loss} />

        <SectionHead aside={probeModel}>{t("migration:history.verdict")}</SectionHead>
        <div className="fcard" style={{ padding: "12px 14px" }}>
          {fail && probeDetail ? (
            <pre className="mono selectable fscroll" style={{ margin: 0, fontSize: 11,
              color: "var(--err-pre)", whiteSpace: "pre-wrap", maxHeight: 320, overflow: "auto",
              lineHeight: 1.5 }}>{probeDetail}</pre>
          ) : probeLines.map((p, i) => (
            <div key={i} style={{ display: "flex", alignItems: "flex-start", gap: 9, fontSize: 12,
              color: "var(--tx2b)", lineHeight: 1.45, marginTop: i ? 8 : 0 }}>
              <span style={{ width: 5, height: 5, borderRadius: "50%", background: probeDot,
                flex: "none", marginTop: 6 }} />{p}
            </div>
          ))}
        </div>

        {rolled && (
          <>
            <SectionHead>{t("migration:history.rollbackTitle")}</SectionHead>
            <div className="fcard" style={{ padding: "12px 14px", display: "flex", gap: 9,
              fontSize: 12, color: "var(--tx2b)", lineHeight: 1.5 }}>
              <span style={{ width: 5, height: 5, borderRadius: "50%", background: "var(--err)",
                flex: "none", marginTop: 6 }} />
              <span>{t("migration:history.rollbackDesc", { tool: TOOL_NAME[h.dst] })}</span>
            </div>
          </>
        )}

        {ok && h.resume && (
          <div style={{ marginTop: 14 }}>
            <CmdRow cmd={h.resume} head={t("migration:history.handoffIn", { tool: TOOL_NAME[h.dst] })} />
          </div>
        )}
      </div>
    </div>
  );
}
