// 快照详情:状态/大小/创建原因/关联会话/变更摘要/差异预览 + 还原入口
import { fmtSize, fmtTime } from "../api.js";
import { ToolIcon } from "../icons.jsx";

export default function SnapshotDetail({ s, restoring, onRestore }) {
  if (!s) return (
    <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
      color: "#9AA3AD", fontSize: 13 }}>还没有快照 —— 会话编辑或迁移前会自动创建</div>
  );
  const state = restoring === "done" ? "已还原" : restoring ? "还原中" : "可还原";
  const stColor = state === "还原中" ? "#E09112" : "#1C9E5A";
  const busy = restoring === true;
  const done = restoring === "done";
  const btnLabel = busy ? "还原中…" : done ? "已还原" : "还原到此快照";
  return (
    <div className="fscroll" style={{ flex: 1, overflowY: "auto", minHeight: 0, animation: "ffade .16s ease" }}>
      <div style={{ padding: "20px 26px 16px", borderBottom: "1px solid #E8ECF0" }}>
        <div style={{ display: "flex", alignItems: "flex-start", gap: 13 }}>
          <ToolIcon tool="claude" size={40} dot={stColor} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 16, fontWeight: 650, letterSpacing: "-.01em" }}>{s.title}</div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "6px 14px", marginTop: 6,
              fontSize: 12, color: "#6B7682" }}>
              <span className="mono" style={{ color: "#8A939D" }}>{s.id}</span>
              <span>{s.trigger}</span>
              <span>{fmtTime(s.time)}</span>
              <span>{fmtSize(s.size)}</span>
            </div>
          </div>
          <button onClick={busy || done ? undefined : onRestore}
            style={{ height: 32, padding: "0 15px", background: busy || done ? "#EEF1F4" : "#0B67F5",
              border: "none", borderRadius: 8, fontSize: 12.5, color: busy || done ? "#9AA3AD" : "#fff",
              cursor: busy || done ? "default" : "pointer", fontWeight: 600, flex: "none" }}>
            {btnLabel}</button>
        </div>
      </div>
      <div style={{ padding: "20px 26px 44px", maxWidth: 760 }}>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10 }}>
          <div style={{ border: "1px solid #E4E9EE", borderRadius: 10, padding: "13px 15px" }}>
            <div style={{ fontSize: 11.5, color: "#8A939D", fontWeight: 600 }}>状态</div>
            <div style={{ display: "inline-flex", alignItems: "center", gap: 6, color: stColor,
              fontWeight: 600, fontSize: 13, marginTop: 7 }}>
              <span style={{ width: 6, height: 6, borderRadius: "50%", background: stColor }} />{state}
            </div>
          </div>
          <div style={{ border: "1px solid #E4E9EE", borderRadius: 10, padding: "13px 15px" }}>
            <div style={{ fontSize: 11.5, color: "#8A939D", fontWeight: 600 }}>快照大小</div>
            <div className="mono" style={{ fontSize: 13, color: "#334155", marginTop: 7 }}>{fmtSize(s.size)}</div>
          </div>
        </div>

        <div style={{ marginTop: 14, border: "1px solid #E4E9EE", borderRadius: 10, padding: "14px 16px" }}>
          <div style={{ fontSize: 11.5, color: "#8A939D", fontWeight: 600 }}>创建原因</div>
          <div style={{ fontSize: 13, color: "#334155", fontWeight: 600, marginTop: 6 }}>
            {s.trigger} · {fmtTime(s.time)}</div>
          <div style={{ fontSize: 12, color: "#6B7682", marginTop: 4, lineHeight: 1.55 }}>
            应用会话编辑或迁移前自动创建的保护快照,可随时还原到该时点状态。</div>
        </div>

        <div style={{ marginTop: 14, border: "1px solid #E4E9EE", borderRadius: 10, padding: "13px 15px",
          display: "flex", alignItems: "center", gap: 11 }}>
          <div style={{ fontSize: 11.5, color: "#8A939D", fontWeight: 600, flex: "none" }}>关联会话</div>
          <span style={{ fontSize: 12.5, color: "#334155", flex: 1, textAlign: "right",
            whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{s.title}</span>
          <span style={{ fontSize: 11.5, color: "#8A939D", flex: "none" }}>Claude Code</span>
        </div>

        <div style={{ fontSize: 12, fontWeight: 600, color: "#6B7682", margin: "18px 0 8px" }}>变更摘要</div>
        <div style={{ border: "1px solid #E4E9EE", borderRadius: 10, padding: "12px 15px" }}>
          {[`完整会话状态:消息、工具调用与文件引用`,
            `快照文件 ${s.path.split("/").pop()} · ${fmtSize(s.size)}`].map((c, i) => (
            <div key={i} style={{ display: "flex", alignItems: "flex-start", gap: 8, fontSize: 12.5,
              color: "#40494F", margin: "5px 0", lineHeight: 1.45 }}>
              <span style={{ width: 5, height: 5, borderRadius: "50%", background: "#8AA0B6",
                flex: "none", marginTop: 6 }} />{c}
            </div>
          ))}
        </div>

        <div style={{ fontSize: 12, fontWeight: 600, color: "#6B7682", margin: "18px 0 8px" }}>差异预览</div>
        <div className="mono" style={{ border: "1px solid #E4E9EE", borderRadius: 10, padding: "12px 14px",
          fontSize: 11.5, lineHeight: 1.7 }}>
          <div style={{ background: "#FBEDEC", color: "#9A3E37", padding: "2px 8px", borderRadius: 5,
            marginBottom: 5 }}>− 当前会话(快照之后的改动将被覆盖)</div>
          <div style={{ background: "#EAF6EE", color: "#2A6B44", padding: "2px 8px", borderRadius: 5 }}>
            + 快照 {s.id} 记录的状态 · {fmtTime(s.time)}</div>
        </div>

        {s.result && !s.result.ok && (
          <div style={{ marginTop: 14, border: "1px solid #EBCBC7", background: "#FDF3F1",
            borderRadius: 10, padding: "13px 15px", fontSize: 12.5, color: "#8A3E37", lineHeight: 1.55 }}>
            <div style={{ fontWeight: 600, marginBottom: 4 }}>上次还原结果</div>{s.result.error}
          </div>
        )}

        <div style={{ fontSize: 11.5, color: "#9AA3AD", marginTop: 14, lineHeight: 1.6 }}>
          还原会先自动创建当前状态的保护快照,再恢复到此快照。若还原后探针未通过,Ferry
          会保持当前状态并在历史中标注,不会写入不完整的产物。</div>
      </div>
    </div>
  );
}
