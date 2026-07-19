import { useState } from "react";
import { TOOL_NAME, fmtSize, fmtTime, resumeCommand, BIG } from "../api.js";
import Badge from "../components/Badge.jsx";
import CopyBtn from "../components/CopyBtn.jsx";
import Spin from "../components/Spin.jsx";

function ToolCallCard({ b }) {
  const [open, setOpen] = useState(false);
  const big = b.size > BIG;
  const label = b.op
    ? `${b.name} · ${String(typeof b.input === "object"
        ? (b.input.command || b.input.file_path || "") : "").slice(0, 80)}`
    : b.name;
  return (
    <div className={`toolcall ${big ? "big" : ""}`}>
      <div className="tool-head" onClick={() => setOpen(!open)}>
        <span className="tool-tag mono">工具调用</span>
        <span className="tool-name mono">{label}</span>
        <span style={{ flex: 1 }} />
        {big && <span className="small" style={{ color: "#A66A00", fontWeight: 600 }}>大输出</span>}
        <span className="small muted">{fmtSize(b.size)} · {open ? "折叠" : "展开"}</span>
      </div>
      {open && (
        <div className="tool-out mono selectable">
          {typeof b.input === "object"
            ? JSON.stringify(b.input, null, 1).slice(0, 2000)
            : String(b.input).slice(0, 2000)}
          {"\n———— 输出 ————\n"}
          {(b.output || "(无输出)").slice(0, 200000)}
        </div>
      )}
    </div>
  );
}

function MessageView({ msg }) {
  const texts = msg.blocks.filter(b => b.kind === "text" && b.text.trim());
  const tools = msg.blocks.filter(b => b.kind === "tool");
  return (
    <>
      {texts.length > 0 && (
        <div className="msg">
          <div className={`avatar ${msg.role}`}>{msg.role === "user" ? "你" : "AI"}</div>
          <div className={`bubble ${msg.role}`}>
            <div className="bubble-head">
              <span className="who">{msg.role === "user" ? "用户消息" : "AI 回复"}</span>
              <span className="sz">{fmtSize(texts.reduce((a, b) => a + b.size, 0))}</span>
            </div>
            <div className="text selectable">{texts.map(t => t.text).join("\n")}</div>
          </div>
        </div>
      )}
      {tools.map((b, i) => <ToolCallCard b={b} key={i} />)}
    </>
  );
}

function findSession(node, id) {
  if (!node || !id || node.id === id) return node;
  for (const child of node.children || []) {
    const found = findSession(child, id);
    if (found) return found;
  }
  return null;
}

function collectCompatibilityIssues(node, depth = 0) {
  if (!node) return [];
  const session = node.title || node.agent_path || (depth ? "子会话" : "主会话");
  return [
    ...(node.loss || []).map(reason => ({ session, id: node.id, reason })),
    ...(node.children || []).flatMap(child => collectCompatibilityIssues(child, depth + 1)),
  ];
}

function SessionTreeNode({ node, selected, onSelect, depth = 0 }) {
  const [open, setOpen] = useState(depth === 0);
  const children = node.children || [];
  return <>
    <div className={`session-node ${selected === node.id ? "on" : ""}`}
      style={{ paddingLeft: 10 + depth * 16 }} onClick={() => onSelect(node.id)}>
      <button className="tree-toggle" onClick={e => {
        e.stopPropagation();
        if (children.length) setOpen(v => !v);
      }}>{children.length ? (open ? "−" : "+") : "·"}</button>
      <div className="session-node-copy">
        <div className="session-node-title">{node.title || (depth ? "子会话" : "主会话")}</div>
        <div className="session-node-meta">{node.count} 条消息{children.length ? ` · ${node.tree_count - 1} 个下级节点` : ""}</div>
      </div>
    </div>
    {open && children.map(child => <SessionTreeNode key={child.id} node={child}
      selected={selected} onSelect={onSelect} depth={depth + 1} />)}
  </>;
}

function Detail({ detail, env, onBack, onMigrate, onEdit }) {
  const { meta, data, error } = detail;
  const [selectedId, setSelectedId] = useState(null);
  const [issuesOpen, setIssuesOpen] = useState(false);
  const selected = findSession(data, selectedId) || data;
  const treeCount = data?.tree_count || meta.tree_count || 1;
  const compatibilityIssues = collectCompatibilityIssues(data);
  const groupedIssues = Object.values(compatibilityIssues.reduce((groups, issue) => {
    const group = groups[issue.reason] || { reason: issue.reason, count: 0, sessions: new Set() };
    group.count += 1;
    group.sessions.add(issue.id);
    groups[issue.reason] = group;
    return groups;
  }, {}));
  const resume = resumeCommand(meta.tool, meta.id, meta.dir);
  return (
    <div className="page">
      <div className="detail-head">
        <div className="crumb"><a onClick={onBack}>会话库</a>
          <span style={{ opacity: .5 }}> / </span>{meta.title || meta.id}</div>
        <div className="detail-title-row">
          <div className="detail-title">
            <Badge tool={meta.tool} />
            <div>
              <div className="h">{meta.title || "(无标题)"}</div>
              <div className="dir mono">{meta.dir || ""}</div>
            </div>
          </div>
          <div style={{ display: "flex", gap: 8, flex: "none" }}>
            <button className="btn primary" onClick={onMigrate}>迁移 →</button>
            {meta.tool === "claude" &&
              <button className="btn" onClick={onEdit} disabled={!data}>会话编辑</button>}
            <CopyBtn text={resume} className="btn" />
          </div>
        </div>
        <div className="detail-meta">
          <span>来源 · <b>{TOOL_NAME[meta.tool]}</b></span>
          <span>更新 · {fmtTime(meta.updated)}</span>
          <span>{meta.count} 条消息</span>
          <span>{fmtSize(meta.size)}</span>
          {treeCount > 1 && <span className="chip native">包含 {treeCount - 1} 个子会话</span>}
          {compatibilityIssues.length > 0 &&
            <button className="chip compatibility" onClick={() => setIssuesOpen(open => !open)}>
              兼容性提示 {compatibilityIssues.length} 项 · {issuesOpen ? "收起" : "查看"}
            </button>}
        </div>
        {issuesOpen && compatibilityIssues.length > 0 &&
          <div className="compatibility-panel">
            <div className="compatibility-intro">
              部分内容无法统一展示，迁移前会再次确认影响。
            </div>
            <div className="compatibility-list">
              {groupedIssues.map(issue =>
                <div className="compatibility-item" key={issue.reason}>
                  <div className="compatibility-reason">
                    <span>{issue.reason}</span>
                    {issue.count > 1 && <b className="compatibility-count">× {issue.count}</b>}
                  </div>
                  <div className="compatibility-scope">涉及 {issue.sessions.size} 个会话节点</div>
                </div>)}
            </div>
          </div>}
      </div>
      <div className="body">
        {error ? <div className="empty">读取失败:{error}</div>
          : !data ? <div className="empty"><Spin /> 解析会话中…</div>
          : <div className={`detail-content ${treeCount > 1 ? "with-tree" : ""}`}>
            {treeCount > 1 && <aside className="session-tree card">
              <div className="session-tree-head">会话树 · {treeCount} 个节点</div>
              <SessionTreeNode node={data} selected={selected.id} onSelect={setSelectedId} />
            </aside>}
            <section className="session-messages">
              {treeCount > 1 && <div className="selected-session-head">
                <div><b>{selected.title || (selected.parent_id ? "子会话" : "主会话")}</b>
                  <span className="mono">{selected.id}</span></div>
                <span>{selected.count} 条消息</span>
              </div>}
              <div className="timeline">{selected.messages.map(m => <MessageView msg={m} key={m.index} />)}</div>
              {selected.messages.length === 0 && <div className="empty">该节点没有可展示的消息</div>}
            </section>
          </div>}
      </div>
    </div>
  );
}

export default Detail;
