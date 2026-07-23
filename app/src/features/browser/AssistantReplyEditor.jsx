import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ACCENT } from "../../domain/tools/toolDisplay.js";
import { CloseIcon, TrashIcon } from "../../components/ui/icons.jsx";

const uid = () => globalThis.crypto?.randomUUID?.() || `item-${Date.now()}-${Math.random()}`;

function SmallButton({ title, danger, disabled, onClick, children }) {
  return <button className={`ficon-btn${danger ? " danger" : ""}`} title={title}
    disabled={disabled} onClick={e => { e.stopPropagation(); onClick(e); }}
    style={{ width: 25, height: 25 }}>{children}</button>;
}

const fieldStyle = {
  width: "100%", boxSizing: "border-box", border: "1px solid var(--line3)",
  borderRadius: 6, background: "var(--surface)", color: "var(--tx2)",
  padding: "7px 9px", fontSize: 12, lineHeight: 1.55, resize: "vertical",
};

function ItemEditor({ item, index, count, onPatch, onRemove, onMove }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(
    item.kind === "text" ? !item.text : !item.name);
  return (
    <div style={{ border: "1px solid var(--line3)", borderRadius: 8,
      background: "var(--fill)", marginBottom: 8, overflow: "hidden" }}>
      <div onClick={() => setExpanded(value => !value)}
        style={{ display: "flex", alignItems: "center", gap: 7, padding: "6px 8px", cursor: "default",
        borderBottom: expanded ? "1px solid var(--line5)" : "none" }}>
        <span style={{ color: "var(--tx4)", fontSize: 11 }}>{expanded ? "▾" : "▸"}</span>
        <span className="mono" style={{ flex: 1, fontSize: 11, fontWeight: 600,
          color: item.kind === "tool" ? "var(--acc-text)" : "var(--tx3b)" }}>
          {item.kind === "tool"
            ? t("browser:replyEditor.toolNamed", { name: item.name || t("browser:replyEditor.toolUnnamed") })
            : t("browser:replyEditor.aiText")}
        </span>
        <SmallButton title={t("browser:replyEditor.moveUp")} disabled={index === 0} onClick={() => onMove(-1)}>↑</SmallButton>
        <SmallButton title={t("browser:replyEditor.moveDown")} disabled={index === count - 1} onClick={() => onMove(1)}>↓</SmallButton>
        <SmallButton title={t("browser:replyEditor.removeItem")} danger onClick={onRemove}><TrashIcon size={11} /></SmallButton>
      </div>
      {expanded && item.kind === "text" && (
        <div style={{ padding: 9 }}>
          <textarea className="fscroll selectable" value={item.text}
            onChange={event => onPatch({ text: event.target.value })}
            placeholder={t("browser:replyEditor.aiTextPlaceholder")} rows={3} style={{ ...fieldStyle, fontFamily: "inherit" }} />
        </div>
      )}
      {expanded && item.kind === "tool" && (
        <div style={{ padding: 9, display: "grid", gap: 8 }}>
          <label style={{ fontSize: 10, color: "var(--tx4)" }}>{t("browser:replyEditor.toolName")}
            <input className="mono selectable" value={item.name}
              onChange={event => onPatch({ name: event.target.value })}
              placeholder="Read / bash / custom_tool" style={{ ...fieldStyle, marginTop: 4 }} />
          </label>
          <label style={{ fontSize: 10, color: "var(--tx4)" }}>
            {t("browser:replyEditor.params")} · <button onClick={() => onPatch({ inputFormat: item.inputFormat === "json" ? "text" : "json" })}
              style={{ border: 0, padding: 0, background: "none", color: ACCENT, cursor: "default",
                fontSize: 10 }}>{item.inputFormat === "json" ? t("browser:replyEditor.jsonFormat") : t("browser:replyEditor.textFormat")}</button>
            <textarea className="mono fscroll selectable" value={item.inputText}
              onChange={event => onPatch({ inputText: event.target.value })}
              rows={4} style={{ ...fieldStyle, marginTop: 4 }} />
          </label>
          <label style={{ fontSize: 10, color: "var(--tx4)" }}>{t("browser:replyEditor.fakeOutput")}
            <textarea className="mono fscroll selectable" value={item.output}
              onChange={event => onPatch({ output: event.target.value })}
              rows={4} style={{ ...fieldStyle, marginTop: 4 }} />
          </label>
        </div>
      )}
    </div>
  );
}

export default function AssistantReplyEditor({ op, onChange, onCancel }) {
  const { t } = useTranslation();
  if (!op) return null;

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
    <div style={{ marginTop: 10, border: `1.5px solid ${ACCENT}`, borderRadius: 10,
      background: "var(--acc-soft5)", padding: 10 }}>
      <div style={{ display: "flex", alignItems: "center", marginBottom: 9 }}>
        <span style={{ fontSize: 12, fontWeight: 650, color: "var(--acc-text)" }}>{t("browser:replyEditor.panelTitle")}</span>
        <span style={{ marginLeft: 8, fontSize: 10, color: "var(--tx4)" }}>{t("browser:replyEditor.panelHint")}</span>
        <button className="ficon-btn" title={t("browser:replyEditor.cancel")} onClick={onCancel}
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
          style={{ height: 27, fontSize: 11 }}>{t("browser:replyEditor.addText")}</button>
        <button className="fbtn" onClick={() => add("tool")}
          style={{ height: 27, fontSize: 11 }}>{t("browser:replyEditor.addTool")}</button>
      </div>
    </div>
  );
}
