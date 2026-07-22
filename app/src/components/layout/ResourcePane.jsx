// 上下文资源栏:三种视图共享同一骨架(标题+搜索/筛选图标/标签/列表/页脚)
import { memo, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ACCENT } from "../../domain/tools/toolDisplay.js";
import { Caret, CloseIcon, FilterIcon, MoreDots, PinIcon,
  SearchIcon, ToolIcon, TrashIcon } from "../ui/icons.jsx";

export function Pane({ collapsed, width, dragging, title, count, placeholder,
  query, onOpenSearch, onClearSearch, filterCount, filterOn, onFilter, footer,
  tokens, listKey, children }) {
  const { t } = useTranslation();
  const w = collapsed ? 0 : width;
  return (
    <div style={{ width: w, flex: "none", overflow: "hidden", background: "var(--pane)",
      borderRight: collapsed ? "none" : "1px solid var(--line)",
      transition: dragging ? "width 0s" : "width .2s ease-out" }}>
      <div style={{ width, height: "100%", display: "flex", flexDirection: "column",
        minHeight: 0, opacity: collapsed ? 0 : 1, transition: "opacity .1s ease" }}>
        {/* 通高侧栏:顶部 44px 归红绿灯,整块可拖拽窗口 */}
        <div data-tauri-drag-region style={{ height: 44, flex: "none" }} />
        <div style={{ flex: "none", padding: "0 10px 0" }}>
          {/* 标题行:名称 + 数量,右侧一排图标(搜索/筛选/排序)——对齐 WorkBuddy 紧凑工具栏 */}
          <div style={{ display: "flex", alignItems: "center", gap: 8, height: 28 }}>
            <span style={{ fontSize: 14, fontWeight: 650, color: "var(--tx1)",
              letterSpacing: "-.01em" }}>{title}</span>
            <span className="mono" style={{ fontSize: 12, color: "var(--tx5)" }}>{count}</span>
            <span style={{ flex: 1 }} />
            <button className="ftool-btn" data-guide="search"
              title={t("app:pane.search")} onClick={onOpenSearch}
              style={query ? { background: "var(--fill4)", color: "var(--tx1)" } : undefined}>
              <SearchIcon /></button>
            <button className="ftool-btn" title={t("app:pane.filterButton")} onClick={onFilter}
              style={{ position: "relative",
                ...(filterOn ? { background: "var(--fill4)", color: "var(--tx1)" } : {}) }}>
              <FilterIcon />
              {filterCount > 0 && (
                <span style={{ position: "absolute", top: 3, right: 3, width: 6, height: 6,
                  borderRadius: "50%", background: ACCENT }} />
              )}</button>
          </div>
          {query && (
            <div style={{ display: "flex", alignItems: "center", gap: 6, height: 26, padding: "0 6px 0 10px",
              background: "var(--acc-soft3)", border: "1px solid var(--acc-line)", borderRadius: 6,
              marginTop: 9, fontSize: 11, color: "var(--acc-text)" }}>
              <SearchIcon />
              <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis",
                whiteSpace: "nowrap" }}>{query}</span>
              <button className="row-act-btn" onClick={onClearSearch}
                title={t("common:empty.clearFilter")}><CloseIcon size={11} /></button>
            </div>
          )}
          {tokens.length > 0 && (
            <div style={{ display: "flex", flexWrap: "wrap", gap: 5, marginTop: 9 }}>
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
          style={{ flex: 1, overflowY: "auto", padding: "8px 8px 10px", minHeight: 0 }}>
          <div key={listKey}>{children}</div>
        </div>
      </div>
    </div>
  );
}

export function PaneEmpty({ text, onClear }) {
  const { t } = useTranslation();
  return (
    <div style={{ textAlign: "center", padding: "34px 12px", color: "var(--tx5)" }}>
      <div style={{ fontSize: 12 }}>{text}</div>
      <button className="fbtn" style={{ marginTop: 10 }} onClick={onClear}>{t("common:empty.clearFilter")}</button>
    </div>
  );
}

// ----- 列表虚拟化:几千行会话全量渲染会拖垮 WebView,只挂载可视区 ± OVERSCAN 内的行 -----
const ROW_H = 30;      // 会话/历史行高(与行内 style 的 height 一致)
const HEADER_H = 24;   // 分组标题行高
const OVERSCAN = 300;  // 视口上下各多渲染的像素,避免快速滚动露白

// 跟踪所在滚动容器的视口(相对本列表顶部的偏移 + 高度),scroll 用 rAF 合帧
function useViewport(ref) {
  const [vp, setVp] = useState({ top: 0, h: 2000 });
  useLayoutEffect(() => {
    const el = ref.current;
    const sc = el?.closest("[data-pane-scroll]");
    if (!sc) return;
    let raf = 0;
    const measure = () => {
      raf = 0;
      const base = el.getBoundingClientRect().top - sc.getBoundingClientRect().top + sc.scrollTop;
      setVp(v => {
        const top = sc.scrollTop - base, h = sc.clientHeight;
        return v.top === top && v.h === h ? v : { top, h };
      });
    };
    const schedule = () => { if (!raf) raf = requestAnimationFrame(measure); };
    measure();
    sc.addEventListener("scroll", schedule, { passive: true });
    const ro = new ResizeObserver(schedule);
    ro.observe(sc);
    return () => { sc.removeEventListener("scroll", schedule); cancelAnimationFrame(raf); ro.disconnect(); };
  }, []);
  return vp;
}

// 平铺后的分组列表:items 为 {key, y, h, node},总高 total,超出视口的行不渲染
function VirtualItems({ items, total }) {
  const ref = useRef(null);
  const { top, h } = useViewport(ref);
  const lo = top - OVERSCAN, hi = top + h + OVERSCAN;
  return (
    <div ref={ref} style={{ position: "relative", height: total }}>
      {items.map(it => (it.y + it.h < lo || it.y > hi) ? null : (
        <div key={it.key} style={{ position: "absolute", top: it.y, left: 0, right: 0, height: it.h }}>
          {it.node}
        </div>
      ))}
    </div>
  );
}

// 选中态:Finder 式整块填充,不描边
const rowSel = on => ({
  background: on ? "var(--acc-soft2)" : "transparent",
});

// 置顶标记:悬浮时隐藏,避免与浮现的置顶按钮重复
const PinGlyph = () => (
  <svg className="row-meta" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke={ACCENT}
    strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" style={{ flex: "none" }}>
    <path d="M12 17v5M9 4h6l1 7 2 2H6l2-2 1-7z" />
  </svg>
);

// 单行会话:紧凑单行,悬浮浮现操作按钮(置顶/删除/更多),右键在列表内禁用
const LibraryRow = memo(function LibraryRow({ r, selected, multi,
  onRowClick, onRowPin, onRowDelete, onRowMore }) {
  const { t } = useTranslation();
  const act = (fn, id) => e => { e.stopPropagation(); fn(id, e); };
  return (
    <div onClick={e => onRowClick(r.id, e)} onContextMenu={e => e.preventDefault()}
      title={r.dir}
      className={selected || multi ? "lib-row" : "lib-row hov-item"}
      style={{ display: "flex", gap: 8, alignItems: "center", padding: "5px 8px", height: 30,
        borderRadius: 6, cursor: "default", transition: "background .12s ease",
        ...rowSel(selected || multi) }}>
      <ToolIcon tool={r.tool} size={18} />
      <span style={{ fontSize: 12, color: "var(--tx1)", whiteSpace: "nowrap",
        overflow: "hidden", textOverflow: "ellipsis", flex: 1, minWidth: 0 }}>{r.title}</span>
      {r.pinned && <PinGlyph />}
      {r.hasMig && <span className="row-meta" title={t("app:library.hasMig")}
        style={{ width: 5, height: 5, borderRadius: "50%",
          background: "var(--info-dot)", flex: "none" }} />}
      <span className="row-meta" style={{ fontSize: 10, color: "var(--tx5)",
        flex: "none" }}>{r.active}</span>
      <span className="row-act" style={{ gap: 1, flex: "none" }}>
        <button className="row-act-btn" onClick={act(onRowPin, r.id)}
          title={r.pinned ? t("app:ctx.unpin") : t("app:ctx.pin")}
          style={r.pinned ? { color: ACCENT } : undefined}>
          <PinIcon filled={r.pinned} /></button>
        <button className="row-act-btn row-act-danger" onClick={act(onRowDelete, r.id)}
          title={t("app:ctx.deleteSession")}>
          <TrashIcon size={13} /></button>
        <button className="row-act-btn" onClick={act(onRowMore, r.id)}
          title={t("app:ctx.more")}><MoreDots /></button>
      </span>
    </div>
  );
});

// 会话库分组列表
export function LibraryList({ groups, collapsed, onToggle, empty, onClear, selectedId, multiSel,
  onRowClick, onRowPin, onRowDelete, onRowMore }) {
  const { t } = useTranslation();
  if (empty) return <PaneEmpty text={t("common:empty.library")} onClear={onClear} />;
  const multiSet = new Set(multiSel);
  const items = [];
  let y = 0;
  groups.forEach(g => {
    const expanded = !(collapsed[g.key] ?? false);
    items.push({ key: `h:${g.key}`, y, h: HEADER_H, node: (
      <div className="hov-row" onClick={() => onToggle(g.key)}
        style={{ display: "flex", alignItems: "center", gap: 5, padding: "0 8px", height: HEADER_H,
          cursor: "default", borderRadius: 6 }}>
        <Caret open={expanded} />
        <span style={{ fontSize: 11, fontWeight: 600, color: "var(--tx3)" }}>{g.label}</span>
        <span style={{ fontSize: 11, color: "var(--tx5)" }}>· {g.count}</span>
      </div>
    ) });
    y += HEADER_H;
    if (expanded) g.rows.forEach(r => {
      items.push({ key: r.id, y, h: ROW_H, node: (
        <LibraryRow r={r} selected={r.id === selectedId} multi={multiSet.has(r.id)}
          onRowClick={onRowClick} onRowPin={onRowPin}
          onRowDelete={onRowDelete} onRowMore={onRowMore} />
      ) });
      y += ROW_H;
    });
    y += 3;
  });
  return <VirtualItems items={items} total={y} />;
}
// 迁移历史分组列表
export function HistoryList({ groups, empty, onClear, onDelete }) {
  const { t } = useTranslation();
  if (empty) return <PaneEmpty text={t("common:empty.history")} onClear={onClear} />;
  const items = [];
  let y = 0;
  groups.forEach(g => {
    items.push({ key: `h:${g.label}`, y, h: HEADER_H, node: (
      <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "0 8px", height: HEADER_H }}>
        <span style={{ fontSize: 11, fontWeight: 600, color: "var(--tx3)" }}>{g.label}</span>
        <span style={{ fontSize: 11, color: "var(--tx5)" }}>· {g.rows.length}</span>
      </div>
    ) });
    y += HEADER_H;
    g.rows.forEach(h => {
      items.push({ key: h.id, y, h: ROW_H, node: (
        <div onClick={h.onClick} onContextMenu={e => e.preventDefault()}
          title={`${h.from} → ${h.to} · ${h.statusLabel ?? h.status}`}
          className={h.selected ? "lib-row" : "lib-row hov-item"}
          style={{ display: "flex", gap: 8, alignItems: "center", padding: "5px 8px", height: 30,
            borderRadius: 6, cursor: "default", transition: "background .12s ease", ...rowSel(h.selected) }}>
          <ToolIcon tool={h.tool} size={18} />
          <span style={{ fontSize: 12, color: "var(--tx1)", whiteSpace: "nowrap",
            overflow: "hidden", textOverflow: "ellipsis", flex: 1, minWidth: 0 }}>{h.title}</span>
          <span className="row-meta" style={{ width: 5, height: 5, borderRadius: "50%",
            background: h.stColor, flex: "none" }} />
          <span className="row-meta"
            style={{ fontSize: 10, color: "var(--tx5)", flex: "none" }}>{h.short}</span>
          {onDelete && h.deletable && (
            <span className="row-act" style={{ gap: 1, flex: "none" }}>
              <button className="row-act-btn row-act-danger" title={t("migration:history.delete")}
                onClick={e => { e.stopPropagation(); onDelete(h.id); }}>
                <TrashIcon size={13} /></button>
            </span>)}
        </div>
      ) });
      y += ROW_H;
    });
    y += 5;
  });
  return <VirtualItems items={items} total={y} />;
}
