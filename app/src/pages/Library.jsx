import { useState } from "react";
import { TOOLS, TOOL_NAME, fmtSize, fmtTime } from "../api.js";
import Badge from "../components/Badge.jsx";
import Spin from "../components/Spin.jsx";
import First from "./First.jsx";

function Library({ scan, scanning, env, onScan, onOpen }) {
  const [q, setQ] = useState("");
  const [tf, setTf] = useState(null);

  if (!scan && !scanning) return <First env={env} onScan={onScan} />;
  if (!scan) {
    return <div className="page"><div className="first">
      <span className="spin" style={{ width: 22, height: 22 }} />
      <div className="big">正在扫描本机会话…</div>
      <div className="desc">首次扫描会解析全部历史文件,之后走缓存,只需零点几秒。</div>
    </div></div>;
  }

  const searchableText = session => [
    session.title, session.dir, session.id,
    ...(session.children || []).map(searchableText),
  ].flat(Infinity).join(" ").toLowerCase();

  const list = (() => {
    const filtered = scan.sessions.filter(s => {
      if (tf && s.tool !== tf) return false;
      if (q) return searchableText(s).includes(q.toLowerCase());
      return true;
    });
    filtered.sort((a, b) => (b.updated || 0) - (a.updated || 0));
    return filtered;
  })();

  const toggleTool = k => {
    setTf(prev => (prev === k ? null : k));
  };

  const chip = k => {
    const info = scan.tools[k] || {};
    const v = (env || {})[k] || {};
    const on = tf === k;
    let cls = "ok", t = TOOL_NAME[k], d = `扫描完成 · 找到 ${info.count ?? 0} 个根会话`;
    if (!v.installed && !info.count) { cls = "miss"; d = "未安装 · 不纳入结果"; }
    else if (info.ok === false) { cls = "warn"; d = `扫描出错:${info.error || ""}`; }
    const disabled = !info.count;
    return (
      <button
        type="button"
        className={`scanchip ${on ? "on" : ""} ${disabled ? "disabled" : ""}`}
        key={k}
        onClick={() => !disabled && toggleTool(k)}
        disabled={disabled}
        title={disabled ? "无可筛选会话" : on ? "取消筛选" : "按此工具筛选"}
      >
        <span className="chip-mark">
          <Badge tool={k} sm />
          <span className={`bigdot ${cls}`} />
        </span>
        <div><div className="t" style={cls === "miss" ? { color: "#78828D" } : {}}>{t}</div>
          <div className="d">{d}</div></div>
      </button>
    );
  };

  return (
    <div className="page">
      <div className="page-head">
        <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between" }}>
          <div>
            <div className="h1">会话库</div>
            <div className="sub">统一汇聚 Claude Code、Codex CLI、OpenCode 的本机会话</div>
          </div>
          <button className="btn" onClick={onScan} disabled={scanning}>
            {scanning ? <><Spin />扫描中</> : "重新扫描"}</button>
        </div>
        <div className="toolbar">
          <div className="search">
            <span className="muted">🔍</span>
            <input placeholder="搜索标题、目录或会话 ID…" value={q}
              onChange={e => setQ(e.target.value)} />
          </div>
          {tf && (
            <button className="btn" onClick={() => setTf(null)}>清除筛选</button>
          )}
        </div>
        <div className="scanbar">{TOOLS.map(chip)}</div>
      </div>
      <div className="body">
        <div className="grid-head">
          <div>工具</div><div>标题 / 目录</div><div>活跃时间</div>
          <div style={{ textAlign: "right" }}>消息数</div>
          <div style={{ textAlign: "right" }}>体积</div><div />
        </div>
        <div className="rows">
          {list.slice(0, 300).map(s => (
            <div className="row" key={`${s.tool}:${s.path || s.id}`} onClick={() => onOpen(s)}>
              <Badge tool={s.tool} />
              <div style={{ minWidth: 0 }}>
                <div className="title">{s.title || "(无标题)"}</div>
                <div className="dir mono">{s.dir || s.id}
                  {s.tree_count > 1 && <span className="tree-hint">包含 {s.tree_count - 1} 个子会话</span>}
                </div>
              </div>
              <div style={{ fontSize: 12.5, color: "#5A6672" }}>{fmtTime(s.updated)}</div>
              <div className="num">{s.count}</div>
              <div className="num">{fmtSize(s.size)}</div>
              <div className="muted">›</div>
            </div>
          ))}
          {list.length === 0 && <div className="empty">没有匹配的会话</div>}
        </div>
        <div className="foot">
          <span>显示 {Math.min(list.length, 300)} / {list.length} 个会话</span>
          <span>{env && !env.opencode.installed ? "OpenCode 未安装,其会话不在结果中" : ""}</span>
        </div>
      </div>
    </div>
  );
}

export default Library;
