// 迁移历史详情:范围/损耗报告/上下文水位/验收结果/回滚信息/接续命令
import { TOOL_NAME, fmtSize, fmtTime, histStatus } from "../api.js";
import { ToolIcon } from "../icons.jsx";
import { CmdRow, LossCols, StatusPill } from "../components/ui.jsx";

const ST_STYLE = {
  "成功": ["var(--ok-bg)", "var(--ok)"],
  "失败": ["var(--err-bg2)", "var(--err)"],
  "已回滚": ["var(--chip)", "var(--tx3b)"],
  "预演": ["var(--warn-bg)", "var(--warn)"],
};

export default function HistoryDetail({ h }) {
  if (!h) return (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
      color: "var(--tx5)", fontSize: 13 }}>还没有迁移记录 —— 在会话详情里点「迁移…」试试</div>
  );
  const status = histStatus(h);
  const [stBg, stColor] = ST_STYLE[status] || ST_STYLE["失败"];
  const ok = status === "成功";
  const fail = status === "失败";
  const rolled = h.rolled_back;
  const range = h.max_turn ? `到第 ${h.max_turn} 轮` : "完整会话";
  const probeDetail = h.probe?.detail || "";
  const probeLines = h.probe
    ? (ok ? probeDetail.split("\n").filter(Boolean).slice(0, 4)
      : probeDetail.split("\n").filter(Boolean))
    : h.validation?.structure
      ? [h.validation.structure.detail, "运行时探针未执行(默认关闭,可在设置中开启)"]
      : ["未运行探针验收"];
  const probeColor = ok ? "var(--ok-deep)" : fail ? "var(--err-deep)" : "var(--tx3b)";
  const probeBg = ok ? "var(--ok-bg)" : fail ? "var(--err-bg)" : "var(--fill3)";
  const probeBorder = ok ? "var(--ok-line)" : fail ? "var(--err-line)" : "var(--line3)";
  const probeModel = h.probe?.model || h.probe_model;

  return (
    <div className="fscroll" style={{ flex: 1, overflowY: "auto", minHeight: 0, animation: "ffade .16s ease" }}>
      <div style={{ padding: "20px 26px 16px", borderBottom: "1px solid var(--line5)" }}>
        <div style={{ display: "flex", alignItems: "flex-start", gap: 13 }}>
          <ToolIcon tool={h.src} size={40} dot={stColor} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 16, fontWeight: 650, letterSpacing: "-.01em" }}>{h.title || h.source_id}</div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "6px 14px", marginTop: 6,
              fontSize: 12, color: "var(--tx3b)" }}>
              <span className="mono" style={{ color: "var(--tx4)" }}>{h.session_id || h.source_id}</span>
              <span>{fmtTime(h.time)}</span>
              <span>{TOOL_NAME[h.src]} → {TOOL_NAME[h.dst]}</span>
            </div>
          </div>
          <StatusPill label={status} color={stColor} bg={stBg} />
        </div>
      </div>
      <div style={{ padding: "20px 26px 44px", maxWidth: 760 }}>
        <div style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "14px 16px",
          display: "flex", flexDirection: "column", gap: 9, fontSize: 12.5 }}>
          <div style={{ display: "flex", justifyContent: "space-between" }}>
            <span style={{ color: "var(--tx4)" }}>迁移范围</span>
            <span style={{ color: "var(--tx2)" }}>{range}{h.msg_count ? ` · ${h.msg_count} 条` : ""}</span>
          </div>
          <div style={{ display: "flex", justifyContent: "space-between" }}>
            <span style={{ color: "var(--tx4)" }}>源工具 → 目标工具</span>
            <span style={{ color: "var(--tx2)" }}>{TOOL_NAME[h.src]} → {TOOL_NAME[h.dst]}</span>
          </div>
          {h.tree_count != null && (
            <div style={{ display: "flex", justifyContent: "space-between" }}>
              <span style={{ color: "var(--tx4)" }}>会话树</span>
              <span style={{ color: "var(--tx2)" }}>{h.tree_count} 个节点{h.topology?.detail ? ` · ${h.topology.detail}` : ""}</span>
            </div>
          )}
        </div>

        <div style={{ fontSize: 12, fontWeight: 600, color: "var(--tx3b)", margin: "18px 0 8px" }}>损耗报告</div>
        <LossCols loss={h.loss} />

        <div style={{ marginTop: 14, border: `1px solid ${probeBorder}`, background: probeBg,
          borderRadius: 10, padding: "13px 15px" }}>
          <div style={{ display: "flex", justifyContent: "space-between", gap: 12 }}>
            <div style={{ fontSize: 12.5, fontWeight: 600, color: probeColor }}>验收结果</div>
            {probeModel && (
              <div className="mono" style={{ fontSize: 11, color: "var(--tx3b)" }}>{probeModel}</div>
            )}
          </div>
          {fail && probeDetail ? (
            <pre className="mono selectable fscroll" style={{ margin: "8px 0 0", fontSize: 11,
              color: "var(--err-pre)", whiteSpace: "pre-wrap", maxHeight: 320, overflow: "auto",
              lineHeight: 1.5 }}>{probeDetail}</pre>
          ) : probeLines.map((p, i) => (
            <div key={i} style={{ display: "flex", alignItems: "flex-start", gap: 8, fontSize: 12,
              color: "var(--tx2b)", marginTop: 7 }}>
              <span style={{ width: 5, height: 5, borderRadius: "50%", background: probeColor,
                flex: "none", marginTop: 6 }} />{p}
            </div>
          ))}
        </div>

        {rolled && (
          <div style={{ marginTop: 14, border: "1px solid var(--err-line)", background: "var(--err-bg)",
            borderRadius: 10, padding: "13px 15px", fontSize: 12.5, color: "var(--err-text)", lineHeight: 1.55 }}>
            <div style={{ fontWeight: 600, marginBottom: 4 }}>回滚信息</div>
            探针未通过,Ferry 已自动回滚,未在 {TOOL_NAME[h.dst]} 保留任何产物。源会话完好,可重试或改用上下文摘要继续。
          </div>
        )}

        {ok && h.resume && (
          <div style={{ marginTop: 14 }}>
            <CmdRow cmd={h.resume} head={`交付 · 在 ${TOOL_NAME[h.dst]} 中接续`} />
          </div>
        )}
      </div>
    </div>
  );
}
