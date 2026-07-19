import { useEffect, useState } from "react";
import { fmtSize, fmtTime, rpc } from "../api.js";
import Spin from "../components/Spin.jsx";

function Snapshots() {
  const [rows, setRows] = useState(null);
  const [busy, setBusy] = useState(null);
  const [msg, setMsg] = useState(null);
  const load = () => rpc("snapshots").then(setRows).catch(() => setRows([]));
  useEffect(() => { load(); }, []);
  const cols = "150px 1fr 110px 180px";
  const total = (rows || []).reduce((a, r) => a + r.size, 0);

  const restore = async r => {
    if (!confirm(`还原会话 ${r.session} 到 ${fmtTime(r.time)} 的快照?\n还原后会运行探针验收(数十秒)。`)) return;
    setBusy(r.path);
    try {
      const res = await rpc("snapshot_restore", { session: r.session });
      setMsg(res.ok ? "还原成功,探针验收通过。" : `还原未生效:${res.error || ""}`);
    } catch (e) { setMsg(`还原失败:${e.message}`); }
    setBusy(null); load();
  };

  const del = async r => {
    if (!confirm("删除该快照?此操作不可恢复。")) return;
    await rpc("snapshot_delete", { path: r.path });
    load();
  };

  return (
    <div className="page">
      <div className="page-head">
        <div className="h1">快照与还原</div>
        <div className="sub">每次编辑前自动创建快照 · 可还原或清理 · 还原经探针验收</div>
      </div>
      <div className="body">
        {msg && <div className={`notice ${msg.includes("成功") ? "ok" : "bad"}`}
          style={{ marginTop: 0, marginBottom: 12 }}>{msg}
          <a style={{ marginLeft: 8 }} onClick={() => setMsg(null)}>关闭</a></div>}
        {!rows ? <div className="empty"><Spin /></div>
          : rows.length === 0 ? <div className="empty">还没有快照。编辑会话时会自动创建。</div>
          : (
            <>
              <div className="small muted" style={{ marginBottom: 10 }}>
                共 {rows.length} 个 · 占用 {fmtSize(total)}</div>
              <div className="table">
                <div className="trow head" style={{ gridTemplateColumns: cols }}>
                  <div>时间</div><div>所属会话</div><div>大小</div><div>操作</div>
                </div>
                {rows.map(r => (
                  <div className="trow" style={{ gridTemplateColumns: cols }} key={r.path}>
                    <div className="muted">{fmtTime(r.time)}</div>
                    <div className="mono small">{r.session}</div>
                    <div>{fmtSize(r.size)}</div>
                    <div style={{ display: "flex", gap: 14 }}>
                      {busy === r.path
                        ? <span className="small muted"><Spin /> 还原并验收中…</span>
                        : <>
                            <a onClick={() => restore(r)}>还原</a>
                            <a style={{ color: "#9E332D" }} onClick={() => del(r)}>清理</a>
                          </>}
                    </div>
                  </div>
                ))}
              </div>
            </>
          )}
        <div className="foot">
          <span>还原会先保住现状;探针验收失败时保持当前状态不变,绝不留下半还原的会话。</span></div>
      </div>
    </div>
  );
}

export default Snapshots;
