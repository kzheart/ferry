// 迁移历史详情:范围/损耗报告/脱敏/上下文水位/探针结果/回滚信息/接续命令
import { TOOL_NAME, fmtSize, fmtTime, histStatus } from "../api.js";
import { ToolIcon } from "../icons.jsx";
import { CmdRow, LossCols, StatusPill } from "../components/ui.jsx";

const ST_STYLE = {
  "成功": ["#EAF7EF", "#1C9E5A"],
  "失败": ["#FBE9E7", "#D5544A"],
  "已回滚": ["#EEF0F2", "#6B7682"],
  "预演": ["#FDF3E6", "#E09112"],
};

export default function HistoryDetail({ h }) {
  if (!h) return (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
      color: "#9AA3AD", fontSize: 13 }}>还没有迁移记录 —— 在会话详情里点「迁移…」试试</div>
  );
  const status = histStatus(h);
  const [stBg, stColor] = ST_STYLE[status] || ST_STYLE["失败"];
  const ok = status === "成功";
  const fail = status === "失败";
  const rolled = h.rolled_back;
  const redactText = h.redacted && Object.keys(h.redacted).length
    ? "已脱敏 " + Object.entries(h.redacted).map(([k, v]) =>
        `${v} 处${{ api_key: "密钥", bearer: "令牌", email: "邮箱" }[k] || k}`).join("、")
    : "未脱敏(可在迁移预演时勾选)";
  const range = h.max_turn ? `到第 ${h.max_turn} 轮` : "完整会话";
  const probeLines = h.probe
    ? h.probe.detail.split("\n").filter(Boolean).slice(0, 4)
    : ["未运行探针验收"];
  const probeColor = ok ? "#1C7C43" : fail ? "#B4433A" : "#6B7682";
  const probeBg = ok ? "#F1FBF5" : fail ? "#FDF3F1" : "#F5F6F7";
  const probeBorder = ok ? "#CDE9D7" : fail ? "#EBCBC7" : "#E4E9EE";

  return (
    <div className="fscroll" style={{ flex: 1, overflowY: "auto", minHeight: 0, animation: "ffade .16s ease" }}>
      <div style={{ padding: "20px 26px 16px", borderBottom: "1px solid #E8ECF0" }}>
        <div style={{ display: "flex", alignItems: "flex-start", gap: 13 }}>
          <ToolIcon tool={h.src} size={40} dot={stColor} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 16, fontWeight: 650, letterSpacing: "-.01em" }}>{h.title || h.source_id}</div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "6px 14px", marginTop: 6,
              fontSize: 12, color: "#6B7682" }}>
              <span className="mono" style={{ color: "#8A939D" }}>{h.session_id || h.source_id}</span>
              <span>{fmtTime(h.time)}</span>
              <span>{TOOL_NAME[h.src]} → {TOOL_NAME[h.dst]}</span>
            </div>
          </div>
          <StatusPill label={status} color={stColor} bg={stBg} />
        </div>
      </div>
      <div style={{ padding: "20px 26px 44px", maxWidth: 760 }}>
        <div style={{ border: "1px solid #E4E9EE", borderRadius: 10, padding: "14px 16px",
          display: "flex", flexDirection: "column", gap: 9, fontSize: 12.5 }}>
          <div style={{ display: "flex", justifyContent: "space-between" }}>
            <span style={{ color: "#8A939D" }}>迁移范围</span>
            <span style={{ color: "#334155" }}>{range}{h.msg_count ? ` · ${h.msg_count} 条` : ""}</span>
          </div>
          <div style={{ display: "flex", justifyContent: "space-between" }}>
            <span style={{ color: "#8A939D" }}>源工具 → 目标工具</span>
            <span style={{ color: "#334155" }}>{TOOL_NAME[h.src]} → {TOOL_NAME[h.dst]}</span>
          </div>
          {h.tree_count != null && (
            <div style={{ display: "flex", justifyContent: "space-between" }}>
              <span style={{ color: "#8A939D" }}>会话树</span>
              <span style={{ color: "#334155" }}>{h.tree_count} 个节点{h.topology?.detail ? ` · ${h.topology.detail}` : ""}</span>
            </div>
          )}
        </div>

        <div style={{ fontSize: 12, fontWeight: 600, color: "#6B7682", margin: "18px 0 8px" }}>损耗报告</div>
        <LossCols loss={h.loss} />

        <div style={{ border: "1px solid #E4E9EE", borderRadius: 10, padding: "13px 15px", marginTop: 14 }}>
          <div style={{ fontSize: 11.5, color: "#8A939D", fontWeight: 600 }}>敏感信息处理</div>
          <div style={{ fontSize: 12.5, color: "#334155", marginTop: 7, lineHeight: 1.5 }}>{redactText}</div>
        </div>

        <div style={{ marginTop: 14, border: `1px solid ${probeBorder}`, background: probeBg,
          borderRadius: 10, padding: "13px 15px" }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: probeColor }}>探针验收结果</div>
          {probeLines.map((p, i) => (
            <div key={i} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12,
              color: "#40494F", marginTop: 7 }}>
              <span style={{ width: 5, height: 5, borderRadius: "50%", background: probeColor,
                flex: "none" }} />{p}
            </div>
          ))}
        </div>

        {rolled && (
          <div style={{ marginTop: 14, border: "1px solid #EBCBC7", background: "#FDF3F1",
            borderRadius: 10, padding: "13px 15px", fontSize: 12.5, color: "#8A3E37", lineHeight: 1.55 }}>
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
