import { useEffect, useState } from "react";
import { TOOL_NAME, fmtTime, rpc } from "../api.js";
import CopyBtn from "../components/CopyBtn.jsx";
import Spin from "../components/Spin.jsx";

function History() {
  const [rows, setRows] = useState(null);
  const [why, setWhy] = useState(null);
  useEffect(() => { rpc("history").then(setRows).catch(() => setRows([])); }, []);
  const cols = "150px 1fr 210px 110px 130px";
  return (
    <div className="page">
      <div className="page-head">
        <div className="h1">迁移历史</div>
        <div className="sub">每一次转出记录都在这里 —— 上次迁移生成的会话去了哪,一目了然</div>
      </div>
      <div className="body">
        {!rows ? <div className="empty"><Spin /></div>
          : rows.length === 0 ? <div className="empty">还没有迁移记录。从会话库选择一个会话开始迁移。</div>
          : (
            <div className="table">
              <div className="trow head" style={{ gridTemplateColumns: cols }}>
                <div>时间</div><div>源会话 → 目标</div><div>损耗摘要</div><div>探针验收</div><div>操作</div>
              </div>
              {rows.map((r, i) => {
                const ok = r.probe && r.probe.ok;
                const failed = r.probe && !r.probe.ok;
                return (
                  <div className="trow" style={{ gridTemplateColumns: cols }} key={i}>
                    <div className="muted">{fmtTime(r.time)}</div>
                    <div style={{ minWidth: 0 }}>
                      <div style={{ fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        {r.title || r.source_id}</div>
                      <div className="small muted">→ {TOOL_NAME[r.dst]} · <span className="mono">
                        {failed ? "未保留产物" : (r.session_id || "").slice(0, 20)}</span></div>
                    </div>
                    <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                      <span className="chip native">原生 {r.loss.native}</span>
                      <span className="chip degrade">降级 {r.loss.degrade}</span>
                      <span className="chip drop">丢弃 {r.loss.drop}</span>
                    </div>
                    <div>{ok ? <span className="tag ok">验收通过</span>
                      : failed ? <span className="tag bad">失败 · 已回滚</span>
                      : <span className="tag warn">未验收</span>}</div>
                    <div style={{ display: "flex", gap: 12 }}>
                      {ok && <CopyBtn text={r.resume} className="btn"
                        style={{ padding: "4px 10px" }} />}
                      {failed && <a onClick={() => setWhy(r)}>查看原因</a>}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        <div className="foot"><span>失败的迁移不会留下任何产物;源会话始终保持只读、可继续使用。</span></div>
      </div>
      {why && (
        <div className="overlay" onClick={() => setWhy(null)}>
          <div className="modal" onClick={e => e.stopPropagation()}>
            <div className="modal-head"><span className="t">失败原因(探针输出)</span>
              <button className="x" onClick={() => setWhy(null)}>✕</button></div>
            <div className="modal-body">
              <div className="mono small selectable" style={{ whiteSpace: "pre-wrap" }}>
                {why.probe ? why.probe.detail : "未知"}</div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default History;
