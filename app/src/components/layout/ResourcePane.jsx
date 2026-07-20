// 上下文资源栏:三种视图共享同一骨架(标题+数量/搜索/筛选/标签/列表/页脚)
import { ACCENT } from "../../domain/tools/toolDisplay.js";
import { Caret, FilterIcon, SearchIcon, SortCaret, ToolIcon } from "../ui/icons.jsx";

export function Pane({ collapsed, width, dragging, title, count, placeholder,
  query, onQuery, filterCount, filterOn, onFilter, sortLabel, footer,
  tokens, listKey, children }) {
  const w = collapsed ? 0 : width;
  return (
    <div style={{ width: w, flex: "none", overflow: "hidden", background: "var(--pane)",
      borderRight: collapsed ? "none" : "1px solid var(--line)",
      transition: dragging ? "width 0s" : "width .2s ease-out" }}>
      <div style={{ width, height: "100%", display: "flex", flexDirection: "column",
        minHeight: 0, opacity: collapsed ? 0 : 1, transition: "opacity .1s ease" }}>
        <div style={{ flex: "none", padding: "13px 14px 0" }}>
          <div style={{ display: "flex", alignItems: "baseline", gap: 9 }}>
            <span style={{ fontSize: 14, fontWeight: 650, color: "var(--tx1)", letterSpacing: "-.01em" }}>{title}</span>
            <span className="mono" style={{ fontSize: 12, color: "var(--tx5)" }}>{count}</span>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 8, height: 30, padding: "0 10px",
            background: "var(--surface)", border: "1px solid var(--line)", borderRadius: 7, marginTop: 11 }}>
            <SearchIcon />
            <input placeholder={placeholder} value={query} onChange={onQuery}
              style={{ border: "none", outline: "none", background: "transparent",
                fontSize: 12.5, flex: 1, color: "var(--tx1)" }} />
          </div>
          <div data-guide="search" style={{ display: "flex", alignItems: "center", gap: 7, marginTop: 8 }}>
            <button onClick={onFilter} style={{ height: 26, display: "flex", alignItems: "center",
              gap: 6, padding: "0 10px", background: filterOn ? "var(--acc-soft)" : "var(--surface)",
              border: `1px solid ${filterOn ? ACCENT : "var(--line)"}`, borderRadius: 7,
              fontSize: 12, color: "var(--tx2)", cursor: "pointer" }}>
              <FilterIcon />筛选{filterCount > 0 ? ` · ${filterCount}` : ""}
            </button>
            <span style={{ flex: 1 }} />
            <span style={{ fontSize: 11.5, color: "var(--tx4)", display: "flex", alignItems: "center", gap: 4 }}>
              {sortLabel}<SortCaret />
            </span>
          </div>
          {tokens.length > 0 && (
            <div style={{ display: "flex", flexWrap: "wrap", gap: 5, marginTop: 9, animation: "fslide .14s ease" }}>
              {tokens.map((tk, i) => (
                <span key={i} style={{ display: "inline-flex", alignItems: "center", gap: 5, height: 22,
                  padding: "0 6px 0 9px", background: "var(--acc-soft3)", border: "1px solid var(--acc-line)",
                  borderRadius: 20, fontSize: 11, color: "var(--acc-text)" }}>
                  {tk.label}
                  <a onClick={tk.onRemove} style={{ color: "var(--acc-mut)", fontSize: 13, lineHeight: 1 }}>×</a>
                </span>
              ))}
            </div>
          )}
        </div>
        <div data-pane-scroll className="fscroll"
          style={{ flex: 1, overflowY: "auto", padding: "10px 8px", minHeight: 0 }}>
          <div key={listKey} style={{ animation: "ffade .14s ease" }}>{children}</div>
        </div>
        <div style={{ flex: "none", borderTop: "1px solid var(--line3)", padding: "9px 13px",
          fontSize: 11, color: "var(--tx5)", background: "var(--pane-foot)", whiteSpace: "nowrap",
          overflow: "hidden", textOverflow: "ellipsis" }}>{footer}</div>
      </div>
    </div>
  );
}

export function PaneEmpty({ text, onClear }) {
  return (
    <div style={{ textAlign: "center", padding: "34px 12px", color: "var(--tx5)" }}>
      <div style={{ fontSize: 12.5 }}>{text}</div>
      <button className="fbtn" style={{ marginTop: 10 }} onClick={onClear}>清除筛选</button>
    </div>
  );
}

const rowSel = on => ({
  background: on ? "var(--acc-soft3)" : "transparent",
  boxShadow: on ? `inset 0 0 0 1px ${ACCENT}` : "none",
});

// 会话库分组列表
export function LibraryList({ groups, empty, onClear }) {
  if (empty) return <PaneEmpty text="没有匹配会话" onClear={onClear} />;
  return groups.map(g => (
    <div key={g.key} style={{ marginBottom: 3 }}>
      <div className="hov-row" onClick={g.onToggle}
        style={{ display: "flex", alignItems: "center", gap: 6, padding: "6px 8px",
          cursor: "pointer", borderRadius: 6 }}>
        <Caret open={g.expanded} />
        <span style={{ fontSize: 11, fontWeight: 600, color: "var(--tx3)" }}>{g.label}</span>
        <span style={{ fontSize: 11, color: "var(--tx5)" }}>· {g.count}</span>
      </div>
      {g.expanded && (
        <div style={{ animation: "fslide .16s ease" }}>
          {g.rows.map(r => (
            <div key={r.id} onClick={r.onClick} onContextMenu={r.onContext} title={r.dir}
              className={r.selected ? undefined : "hov-item"}
              style={{ display: "flex", gap: 10, alignItems: "center", padding: "8px 10px",
                borderRadius: 8, cursor: "pointer", transition: "background .12s ease,box-shadow .12s ease",
                ...rowSel(r.selected) }}>
              <ToolIcon tool={r.tool} size={22} dot={r.dot} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                  <span style={{ fontSize: 12.5, fontWeight: 550, color: "var(--tx1)", whiteSpace: "nowrap",
                    overflow: "hidden", textOverflow: "ellipsis", flex: 1 }}>{r.title}</span>
                  <span style={{ fontSize: 10.5, color: "var(--tx5)", flex: "none" }}>{r.active}</span>
                </div>
                <div style={{ display: "flex", alignItems: "center", gap: 7, marginTop: 3 }}>
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--tx5)", whiteSpace: "nowrap",
                    overflow: "hidden", textOverflow: "ellipsis" }}>{r.repo}</span>
                  {r.hasSub && <span style={{ fontSize: 10, color: "var(--tx3b)", background: "var(--chip)",
                    borderRadius: 4, padding: "0 5px", flex: "none", whiteSpace: "nowrap" }}>{r.subLabel}</span>}
                  {r.hasMig && <span title="含迁移记录" style={{ width: 5, height: 5, borderRadius: "50%",
                    background: "var(--info-dot)", flex: "none" }} />}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  ));
}

// 迁移历史分组列表
export function HistoryList({ groups, empty, onClear }) {
  if (empty) return <PaneEmpty text="没有匹配迁移记录" onClear={onClear} />;
  return groups.map(g => (
    <div key={g.label} style={{ marginBottom: 5 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "6px 8px 4px" }}>
        <span style={{ fontSize: 11, fontWeight: 600, color: "var(--tx3)" }}>{g.label}</span>
        <span style={{ fontSize: 11, color: "var(--tx5)" }}>· {g.rows.length}</span>
      </div>
      {g.rows.map(h => (
        <div key={h.id} onClick={h.onClick}
          className={h.selected ? undefined : "hov-item"}
          style={{ display: "flex", gap: 10, alignItems: "center", padding: "8px 10px", borderRadius: 8,
            cursor: "pointer", transition: "background .12s ease", ...rowSel(h.selected) }}>
          <ToolIcon tool={h.tool} size={22} dot={h.stColor} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)", whiteSpace: "nowrap",
                overflow: "hidden", textOverflow: "ellipsis", flex: 1 }}>{h.title}</span>
              <span style={{ fontSize: 10.5, color: "var(--tx5)", flex: "none" }}>{h.short}</span>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 2 }}>
              <span style={{ fontSize: 10.5, color: "var(--tx4)", whiteSpace: "nowrap", overflow: "hidden",
                textOverflow: "ellipsis" }}>{h.from} → {h.to}</span>
              <span style={{ width: 5, height: 5, borderRadius: "50%", background: h.stColor, flex: "none" }} />
              <span style={{ fontSize: 10.5, color: h.stColor, flex: "none" }}>{h.status}</span>
            </div>
          </div>
        </div>
      ))}
    </div>
  ));
}

// 快照列表
export function SnapList({ rows, empty, onClear }) {
  if (empty) return <PaneEmpty text="没有匹配快照" onClear={onClear} />;
  return rows.map(s => (
    <div key={s.id} onClick={s.onClick}
      className={s.selected ? undefined : "hov-item"}
      style={{ display: "flex", gap: 10, alignItems: "center", padding: "8px 10px", borderRadius: 8,
        cursor: "pointer", transition: "background .12s ease", ...rowSel(s.selected) }}>
      <ToolIcon tool={s.tool} size={22} dot={s.stColor} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <span className="mono" style={{ fontSize: 11.5, fontWeight: 600, color: "var(--tx2)",
            whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis", flex: 1 }}>{s.id}</span>
          <span style={{ fontSize: 10.5, color: "var(--tx5)", flex: "none" }}>{s.short}</span>
        </div>
        <div style={{ marginTop: 2, fontSize: 11, color: "var(--tx1)", whiteSpace: "nowrap",
          overflow: "hidden", textOverflow: "ellipsis" }}>{s.title}</div>
        <div style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 2 }}>
          <span style={{ fontSize: 10.5, color: "var(--tx4)" }}>{s.trigger}</span>
          <span style={{ width: 4, height: 4, borderRadius: "50%", background: s.stColor, flex: "none" }} />
          <span style={{ fontSize: 10.5, color: s.stColor }}>{s.status}</span>
        </div>
      </div>
    </div>
  ));
}
