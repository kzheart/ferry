import { useState } from "react";
import { ACCENT } from "../../domain/tools/toolDisplay.js";
import { CloseIcon, PencilIcon, TrashIcon } from "../../components/ui/icons.jsx";

const uid = () => globalThis.crypto?.randomUUID?.() || `item-${Date.now()}-${Math.random()}`;

function SmallButton({ title, danger, disabled, onClick, children }) {
  return <button className={`ficon-btn${danger ? " danger" : ""}`} title={title}
    disabled={disabled} onClick={onClick} style={{ width: 25, height: 25 }}>{children}</button>;
}

const fieldStyle = {
  width: "100%", boxSizing: "border-box", border: "1px solid var(--line3)",
  borderRadius: 7, background: "var(--surface)", color: "var(--tx2)",
  padding: "7px 9px", fontSize: 12, lineHeight: 1.55, outline: "none", resize: "vertical",
};

function ItemEditor({ item, index, count, onPatch, onRemove, onMove }) {
  const [expanded, setExpanded] = useState(
    item.kind === "text" ? !item.text : !item.name);
  return (
    <div style={{ border: "1px solid var(--line3)", borderRadius: 9,
      background: "var(--fill)", marginBottom: 8, overflow: "hidden" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 7, padding: "6px 8px",
        borderBottom: expanded ? "1px solid var(--line5)" : "none" }}>
        <button onClick={() => setExpanded(value => !value)} style={{ border: 0, background: "none",
          color: "var(--tx4)", cursor: "pointer", fontSize: 11 }}>{expanded ? "▾" : "▸"}</button>
        <span className="mono" style={{ flex: 1, fontSize: 11.5, fontWeight: 600,
          color: item.kind === "tool" ? "var(--acc-text)" : "var(--tx3b)" }}>
          {item.kind === "tool" ? `工具 · ${item.name || "未命名"}` : "AI 文本"}
        </span>
        <SmallButton title="上移" disabled={index === 0} onClick={() => onMove(-1)}>↑</SmallButton>
        <SmallButton title="下移" disabled={index === count - 1} onClick={() => onMove(1)}>↓</SmallButton>
        <SmallButton title="删除内容块" danger onClick={onRemove}><TrashIcon size={11} /></SmallButton>
      </div>
      {expanded && item.kind === "text" && (
        <div style={{ padding: 9 }}>
          <textarea className="fscroll selectable" value={item.text}
            onChange={event => onPatch({ text: event.target.value })}
            placeholder="AI 回复文本" rows={3} style={{ ...fieldStyle, fontFamily: "inherit" }} />
        </div>
      )}
      {expanded && item.kind === "tool" && (
        <div style={{ padding: 9, display: "grid", gap: 8 }}>
          <label style={{ fontSize: 10.5, color: "var(--tx4)" }}>工具名称
            <input className="mono selectable" value={item.name}
              onChange={event => onPatch({ name: event.target.value })}
              placeholder="Read / bash / custom_tool" style={{ ...fieldStyle, marginTop: 4 }} />
          </label>
          <label style={{ fontSize: 10.5, color: "var(--tx4)" }}>
            参数 · <button onClick={() => onPatch({ inputFormat: item.inputFormat === "json" ? "text" : "json" })}
              style={{ border: 0, padding: 0, background: "none", color: ACCENT, cursor: "pointer",
                fontSize: 10.5 }}>{item.inputFormat === "json" ? "JSON" : "原始文本"}</button>
            <textarea className="mono fscroll selectable" value={item.inputText}
              onChange={event => onPatch({ inputText: event.target.value })}
              rows={4} style={{ ...fieldStyle, marginTop: 4 }} />
          </label>
          <label style={{ fontSize: 10.5, color: "var(--tx4)" }}>伪造的工具输出
            <textarea className="mono fscroll selectable" value={item.output}
              onChange={event => onPatch({ output: event.target.value })}
              rows={4} style={{ ...fieldStyle, marginTop: 4 }} />
          </label>
        </div>
      )}
    </div>
  );
}

export default function AssistantReplyEditor({ op, blocked, canAuthor, onStart, onChange, onCancel }) {
  if (!op) {
    return (
      <button className="fbtn" disabled={!canAuthor || blocked} onClick={onStart}
        title={blocked ? "请先应用或放弃其他暂存操作" : "修改或伪造 AI 回复与工具调用"}
        style={{ height: 27, padding: "0 10px", fontSize: 11.5, color: ACCENT }}>
        <PencilIcon size={11} /> 编排 AI 回复
      </button>
    );
  }

  const patchItem = (id, patch) => onChange(op.items.map(item => item.id === id ? { ...item, ...patch } : item));
  const removeItem = id => onChange(op.items.filter(item => item.id !== id));
  const moveItem = (index, direction) => {
    const target = index + direction;
    if (target < 0 || target >= op.items.length) return;
    const items = [...op.items];
    [items[index], items[target]] = [items[target], items[index]];
    onChange(items);
  };
  const add = kind => onChange([...op.items, kind === "text"
    ? { id: uid(), kind: "text", text: "" }
    : { id: uid(), kind: "tool", name: "", inputText: "{}", inputFormat: "json", output: "" }]);

  return (
    <div style={{ marginTop: 10, border: `1.5px solid ${ACCENT}`, borderRadius: 11,
      background: "var(--acc-soft5)", padding: 10 }}>
      <div style={{ display: "flex", alignItems: "center", marginBottom: 9 }}>
        <span style={{ fontSize: 12, fontWeight: 650, color: "var(--acc-text)" }}>AI 回复编排</span>
        <span style={{ marginLeft: 8, fontSize: 10.5, color: "var(--tx4)" }}>仅修改历史，不会执行工具</span>
        <button className="ficon-btn" title="放弃 AI 回复修改" onClick={onCancel}
          style={{ marginLeft: "auto" }}><CloseIcon size={11} /></button>
      </div>
      <div className="fscroll" style={{ maxHeight: 560, overflowY: "auto", paddingRight: 3 }}>
        {op.items.map((item, index) => (
          <ItemEditor key={item.id} item={item} index={index} count={op.items.length}
            onPatch={patch => patchItem(item.id, patch)} onRemove={() => removeItem(item.id)}
            onMove={direction => moveItem(index, direction)} />
        ))}
      </div>
      <div style={{ display: "flex", gap: 7, marginTop: 8 }}>
        <button className="fbtn" onClick={() => add("text")}
          style={{ height: 27, fontSize: 11.5 }}>+ 文本</button>
        <button className="fbtn" onClick={() => add("tool")}
          style={{ height: 27, fontSize: 11.5 }}>+ 工具调用</button>
      </div>
    </div>
  );
}
