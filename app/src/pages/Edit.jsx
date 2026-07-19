import { useMemo, useState } from "react";
import { fmtSize, openTerminal, rpc, BIG } from "../api.js";
import Cmd from "../components/Cmd.jsx";
import Spin from "../components/Spin.jsx";

function editTurns(data) {
  const turns = [];
  let cur = null;
  for (const m of data.messages) {
    if (m.role === "user" || !cur) { cur = { msgs: [] }; turns.push(cur); }
    cur.msgs.push(m);
  }
  return turns;
}

function RewriteModal({ data, initial, onClose, onOk }) {
  const cands = data.messages.filter(m => m.uuid &&
    m.blocks.some(b => b.kind === "text" && b.text.trim()));
  const [idx, setIdx] = useState(0);
  const textOf = m => m.blocks.find(b => b.kind === "text").text;
  const [text, setText] = useState(cands.length ? textOf(cands[0]) : "");
  return (
    <div className="overlay">
      <div className="modal">
        <div className="modal-head"><span className="t">改写单条消息</span>
          <button className="x" onClick={onClose}>✕</button></div>
        <div className="modal-body">
          <label className="f">选择消息</label>
          <select value={idx} onChange={e => {
            const i = +e.target.value; setIdx(i); setText(textOf(cands[i]));
          }}>
            {cands.map((m, i) => (
              <option value={i} key={m.uuid}>
                #{m.index} {m.role === "user" ? "用户" : "助手"} · {textOf(m).slice(0, 60)}
              </option>
            ))}
          </select>
          <label className="f">新文本</label>
          <textarea rows={6} value={text} onChange={e => setText(e.target.value)} />
        </div>
        <div className="modal-foot">
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn primary" disabled={!cands.length}
            onClick={() => onOk({ uuid: cands[idx].uuid, text })}>暂存改写</button>
        </div>
      </div>
    </div>
  );
}

function Edit({ edit, setEdit, onBack, onBackDetail, gotoSnapshots, afterApply }) {
  const { meta, ref, data } = edit;
  const [modal, setModal] = useState(false);
  const turns = useMemo(() => editTurns(data), [data]);

  const ops = useMemo(() => {
    const o = [...edit.delTurns].sort((a, b) => b - a)
      .map(t => ({ op: "delete-turn", turn: t + 1 }));
    if (edit.truncate) o.push({ op: "truncate", threshold: edit.threshold });
    if (edit.rewrite) o.push({ op: "rewrite", uuid: edit.rewrite.uuid, text: edit.rewrite.text });
    return o;
  }, [edit]);

  const preview = async () => {
    setEdit(e => ({ ...e, preview: "loading" }));
    try {
      const p = await rpc("edit_preview", { ref, ops });
      setEdit(e => ({ ...e, preview: p }));
    } catch (err) { setEdit(e => ({ ...e, preview: { error: err.message } })); }
  };

  const apply = async () => {
    setEdit(e => ({ ...e, applying: true }));
    try {
      const result = await rpc("edit_apply", { ref, ops });
      setEdit(e => ({ ...e, applying: false, result }));
      afterApply();
    } catch (err) {
      setEdit(e => ({ ...e, applying: false, result: { ok: false, error: err.message } }));
    }
  };

  if (edit.result) {
    const r = edit.result;
    return (
      <div className="page">
        <div className="page-head">
          <div className="crumb"><a onClick={onBack}>会话库</a>
            <span style={{ opacity: .5 }}> / </span>会话编辑</div>
          <div className="h1">{r.ok ? "编辑完成" : "编辑未生效"}</div>
        </div>
        <div className="body">
          <div style={{ maxWidth: 680, display: "flex", flexDirection: "column", gap: 16 }}>
            {r.ok ? (
              <>
                <div className="notice ok" style={{ margin: 0 }}>
                  已应用:{(r.notes || []).join(" · ")} · 探针验收通过</div>
                <div className="card" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 12 }}>
                  <div className="small muted">快照:<span className="mono">{r.snapshot}</span>
                    (可在「快照与还原」回退)</div>
                  <Cmd text={r.resume} />
                  <div style={{ display: "flex", gap: 10 }}>
                    <button className="btn primary" onClick={() => openTerminal(r.resume)}>在终端打开</button>
                    <button className="btn" onClick={gotoSnapshots}>查看快照</button>
                  </div>
                </div>
              </>
            ) : (
              <>
                <div className="notice bad" style={{ margin: 0 }}>
                  {r.error || "失败"}{r.snapshot ? " · 已自动还原快照,会话保持原状" : ""}</div>
                {r.probe && <div className="card" style={{ padding: 16 }}>
                  <div className="small mono selectable" style={{ whiteSpace: "pre-wrap" }}>
                    {r.probe.detail}</div></div>}
              </>
            )}
            <div><button className="btn" onClick={onBack}>返回会话库</button></div>
          </div>
        </div>
      </div>
    );
  }

  const pv = edit.preview;
  return (
    <div className="page">
      <div className="detail-head" style={{ paddingBottom: 14 }}>
        <div className="crumb">
          <a onClick={onBack}>会话库</a><span style={{ opacity: .5 }}> / </span>
          <a onClick={onBackDetail}>{meta.title || meta.id}</a>
          <span style={{ opacity: .5 }}> / </span>会话编辑</div>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
          <div className="h1" style={{ fontSize: 21 }}>会话编辑</div>
          <div className="small muted">点击轮次标记删除 · 右侧配置裁剪与改写</div>
        </div>
      </div>
      <div className="edit-layout">
        <div className="edit-list">
          <div className="small" style={{ fontWeight: 700, color: "#8A94A0" }}>
            多轮历史 · 共 {turns.length} 轮,点击选中/取消删除</div>
          {turns.map((t, i) => {
            const del = edit.delTurns.has(i);
            const first = t.msgs[0].blocks.find(b => b.kind === "text");
            const sz = t.msgs.reduce((a, mm) =>
              a + mm.blocks.reduce((x, b) => x + (b.size || 0), 0), 0);
            const tools = t.msgs.flatMap(mm => mm.blocks.filter(b => b.kind === "tool"));
            const bigTool = tools.some(b => b.size > BIG);
            return (
              <div className={`turn ${del ? "del" : ""}`} key={i}
                onClick={() => setEdit(e => {
                  const n = new Set(e.delTurns);
                  n.has(i) ? n.delete(i) : n.add(i);
                  return { ...e, delTurns: n, preview: null };
                })}>
                <span className="cb">{del ? "✓" : ""}</span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div className="t">第 {i + 1} 轮 · 用户 → 助手{del ? " · 已标记删除" : ""}</div>
                  <div className="d">{(first ? first.text : "(工具轮)").slice(0, 90)}</div>
                  {tools.length > 0 && (
                    <div className="d">{tools.length} 次工具调用
                      {bigTool && <span style={{ color: "#A66A00", fontWeight: 600 }}> · 含大输出</span>}
                    </div>
                  )}
                </div>
                <span className="small muted">{fmtSize(sz)}</span>
              </div>
            );
          })}
        </div>
        <div className="edit-side">
          <div style={{ fontSize: 13, fontWeight: 700 }}>已暂存 {ops.length} 项操作</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 7, fontSize: 12, color: "#5A6672" }}>
            {[...edit.delTurns].sort((a, b) => a - b).map(t =>
              <div key={t}><span className="sq drop" style={{ marginRight: 7 }} />删除轮次 · 第 {t + 1} 轮</div>)}
            {edit.truncate &&
              <div><span className="sq degrade" style={{ marginRight: 7 }} />裁剪超过 {edit.threshold} 字符的工具输出</div>}
            {edit.rewrite &&
              <div><span className="sq native" style={{ marginRight: 7 }} />改写 1 条消息</div>}
            {ops.length === 0 && <div className="muted">尚未暂存任何操作</div>}
          </div>
          <div className="card" style={{ padding: 13, display: "flex", flexDirection: "column", gap: 9 }}>
            <label className="confirm" style={{ fontSize: 12.5 }}>
              <input type="checkbox" checked={edit.truncate}
                onChange={e => setEdit(x => ({ ...x, truncate: e.target.checked, preview: null }))} />
              <span>裁剪超大工具输出</span>
            </label>
            <div>
              <label className="f">保留阈值(字符)</label>
              <input type="number" min={256} step={256} value={edit.threshold}
                onChange={e => setEdit(x => ({ ...x, threshold: +e.target.value || 4096, preview: null }))} />
            </div>
          </div>
          <div className="card" style={{ padding: 13, display: "flex", flexDirection: "column", gap: 9 }}>
            <div className="f" style={{ margin: 0 }}>改写单条消息</div>
            <button className="btn" onClick={() => setModal(true)}>
              {edit.rewrite ? "重新选择消息…" : "选择消息并改写…"}</button>
          </div>
          <button className="btn" disabled={!ops.length} onClick={preview}>计算差异预览</button>
          {pv === "loading" && <div className="small muted"><Spin /> 计算中…</div>}
          {pv && pv !== "loading" && !pv.error && (
            <div className="card" style={{ padding: 14, display: "flex", flexDirection: "column", gap: 12 }}>
              <div className="kv"><span className="k">消息数</span>
                <span className="v">{pv.before.count} <span className="muted">→</span> <b>{pv.after.count}</b></span></div>
              <div className="kv"><span className="k">总体积</span>
                <span className="v">{fmtSize(pv.before.size)} <span className="muted">→</span> <b>{fmtSize(pv.after.size)}</b>
                  <span style={{ color: "#0F9D7A", fontWeight: 600 }}> −{fmtSize(pv.before.size - pv.after.size)}</span></span></div>
              <div className="small muted">{pv.notes.join(" · ")}</div>
            </div>
          )}
          {pv && pv.error && <div className="notice bad" style={{ margin: 0 }}>{pv.error}</div>}
          <div style={{ flex: 1 }} />
          <div className="small muted" style={{ lineHeight: 1.5 }}>
            应用前会自动创建快照;若探针验收未通过将自动还原,绝不留下坏会话。</div>
          <button className="btn primary" disabled={!ops.length || edit.applying} onClick={apply}
            style={{ justifyContent: "center", padding: 12 }}>
            {edit.applying ? <><Spin /> 应用并验收中(数十秒)…</> : "应用编辑(快照保护)"}</button>
        </div>
      </div>
      {modal && <RewriteModal data={data} onClose={() => setModal(false)}
        onOk={rw => { setEdit(e => ({ ...e, rewrite: rw, preview: null })); setModal(false); }} />}
    </div>
  );
}

export default Edit;
