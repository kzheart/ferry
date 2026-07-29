// 上下文资源栏:三种视图共享同一骨架(标题+搜索/筛选图标/标签/列表/页脚)
import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ACCENT } from "../shared/ui/toolDisplay.js";
import { supportsAgentCapability } from "../shared/contracts/tools.js";
import { Caret, CloseIcon, FilterIcon, MoreDots, PinIcon,
  SearchIcon, ToolIcon, TrashIcon } from "../shared/ui/icons.jsx";

export function Pane({ collapsed, width, dragging, title, count,
  query, onOpenSearch, onClearSearch, filterCount, filterOn, onFilter,
  tokens, listKey, children }) {
  const { t } = useTranslation();
  const w = collapsed ? 0 : width;
  return (
    <div data-guide="pane"
      style={{ width: w, flex: "none", overflow: "hidden", background: "var(--pane)",
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
            <button className="ftool-btn" data-guide="filter"
              title={t("app:pane.filterButton")} onClick={onFilter}
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

// 空态分三种:被筛掉了、本来就没有、扫描失败。
// 首次启动一个会话都没扫到的人,不该看到"没有匹配"和一个点了没反应的清除筛选;
// 扫描失败的人更不该——那等于把故障说成了搜索条件太严。
function PaneEmpty({ text, hint, detail, actions = [] }) {
  const live = actions.filter(Boolean);
  return (
    <div style={{ textAlign: "center", padding: "34px 12px", color: "var(--tx5)" }}>
      <div style={{ fontSize: 12 }}>{text}</div>
      {hint && <div style={{ fontSize: 11, marginTop: 6, lineHeight: 1.55, opacity: .85 }}>{hint}</div>}
      {detail && (
        <div className="mono selectable" style={{ fontSize: 10, marginTop: 8, padding: "6px 8px",
          background: "var(--err-bg)", border: "1px solid var(--err-line)", borderRadius: 6,
          color: "var(--err-text)", textAlign: "left", lineHeight: 1.5,
          wordBreak: "break-word" }}>{detail}</div>
      )}
      {live.length > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: "center",
          marginTop: 11 }}>
          {live.map(a => (
            <button key={a.label} className={a.primary ? "fbtn fbtn-primary" : "fbtn"}
              onClick={a.onClick}>{a.label}</button>
          ))}
        </div>
      )}
    </div>
  );
}

// 扫描失败但上次的结果还在:列表照常能用,但不说一声就等于让用户对着旧数据
// 以为是最新的——尤其是刚新建了会话却怎么也刷不出来的时候。
export function StaleScanNotice({ error, scanning, onRescan }) {
  const { t } = useTranslation();
  return (
    <div style={{ margin: "0 2px 8px", padding: "8px 9px", borderRadius: 7,
      background: "var(--err-bg)", border: "1px solid var(--err-line)" }}>
      <div style={{ fontSize: 11, color: "var(--err-text)", lineHeight: 1.5 }}>
        {t("common:empty.staleScan")}
      </div>
      <div className="mono selectable" style={{ fontSize: 10, marginTop: 4, opacity: .8,
        color: "var(--err-text)", wordBreak: "break-word", lineHeight: 1.45 }}>{error}</div>
      {onRescan && (
        <button className="fbtn" disabled={scanning} onClick={onRescan}
          style={{ marginTop: 7, height: 24, fontSize: 11 }}>
          {t(scanning ? "common:empty.retryingScan" : "common:empty.retryScan")}
        </button>
      )}
    </div>
  );
}

// ----- 列表虚拟化:几千行会话全量渲染会拖垮 WebView,只挂载可视区 ± OVERSCAN 内的行 -----
const ROW_H = 30;      // 会话/历史行高(与行内 style 的 height 一致)
const HEADER_H = 24;   // 分组标题行高
const OVERSCAN = 300;  // 视口上下各多渲染的像素,避免快速滚动露白

// 列表顶部相对滚动容器内容原点的偏移:虚拟化的 y 都以此为基准
function listBase(el, sc) {
  return el.getBoundingClientRect().top - sc.getBoundingClientRect().top + sc.scrollTop;
}

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
      const base = listBase(el, sc);
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

// 选中行滚动跟随:列表是虚拟化的,视口外的行根本没挂载,
// 键盘 ↑/↓ 翻过视口后光靠 DOM 找不到目标,只能按 y 直接算滚动位置。
const SCROLL_PAD = 8;

/**
 * 让目标行进入视口所需的 scrollTop;已经在视口内则返回 null(不打断用户滚动)。
 * 语义对齐 scrollIntoView({ block: "nearest" }):上方超出就贴上沿,下方超出就贴下沿。
 * 纯函数——jsdom 不做布局,滚动跟随的正确性只能在这里断言。
 */
export function nextFocusScrollTop({ itemTop, itemHeight, scrollTop, viewHeight }) {
  const itemBottom = itemTop + itemHeight;
  if (itemTop < scrollTop + SCROLL_PAD) {
    return Math.max(0, itemTop - SCROLL_PAD);
  }
  if (itemBottom > scrollTop + viewHeight - SCROLL_PAD) {
    return Math.max(0, itemBottom - viewHeight + SCROLL_PAD);
  }
  return null;
}

function useFocusScroll(ref, items, focusKey) {
  const itemsRef = useRef(items);
  itemsRef.current = items;
  useEffect(() => {
    if (!focusKey) return;
    const el = ref.current;
    const sc = el?.closest("[data-pane-scroll]");
    if (!sc) return;
    const item = itemsRef.current.find(it => it.key === focusKey);
    if (!item) return; // 被筛掉或所在分组已折叠
    const next = nextFocusScrollTop({
      itemTop: listBase(el, sc) + item.y,
      itemHeight: item.h,
      scrollTop: sc.scrollTop,
      viewHeight: sc.clientHeight,
    });
    if (next !== null) sc.scrollTop = next;
  }, [focusKey]);
}

// 平铺后的分组列表:items 为 {key, y, h, node},总高 total,超出视口的行不渲染
function VirtualItems({ items, total, focusKey }) {
  const ref = useRef(null);
  const { top, h } = useViewport(ref);
  useFocusScroll(ref, items, focusKey);
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

// 单行会话:紧凑单行,悬浮浮现操作按钮(置顶/删除/更多);双击标题就地重命名
const LibraryRow = memo(function LibraryRow({ r, selected, multi, editing,
  onRowClick, onRowPin, onRowDelete, onRowMore,
  onRowRename, onRowRenameSubmit, onRowRenameCancel }) {
  const { t } = useTranslation();
  const act = (fn, key) => e => { e.stopPropagation(); fn(key, e); };
  // Enter/Esc 已经了结这次重命名后,紧随的 blur 不能再触发一次提交
  const settled = useRef(false);

  if (editing) {
    return (
      <div className="lib-row" style={{ display: "flex", gap: 8, alignItems: "center",
        padding: "5px 8px", height: 30, borderRadius: 6, background: "var(--acc-soft2)" }}>
        <ToolIcon tool={r.tool} size={18} />
        <input autoFocus defaultValue={r.title}
          placeholder={t("app:prompt.renamePlaceholder")}
          onFocus={e => { settled.current = false; e.target.select(); }}
          // 失焦即提交(对齐 Finder):点向别处不该丢掉刚输入的名字
          onBlur={e => {
            if (settled.current) return;
            settled.current = true;
            onRowRenameSubmit(r.key, e.currentTarget.value);
          }}
          onClick={e => e.stopPropagation()}
          onKeyDown={e => {
            e.stopPropagation();
            if (e.key === "Enter") {
              settled.current = true;
              onRowRenameSubmit(r.key, e.currentTarget.value);
            } else if (e.key === "Escape") {
              settled.current = true;
              onRowRenameCancel();
            }
          }}
          style={{ flex: 1, minWidth: 0, height: 20, border: "none", outline: "none",
            background: "transparent", color: "var(--tx1)", fontSize: 12, padding: 0 }} />
      </div>
    );
  }

  return (
    <div onClick={e => onRowClick(r.key, e)}
      onDoubleClick={e => { if (!e.target.closest(".row-act")) onRowRename(r.key); }}
      onContextMenu={e => { e.preventDefault(); e.stopPropagation(); onRowMore(r.key, e); }}
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
        <button className="row-act-btn" onClick={act(onRowPin, r.key)}
          title={r.pinned ? t("app:ctx.unpin") : t("app:ctx.pin")}
          style={r.pinned ? { color: ACCENT } : undefined}>
          <PinIcon filled={r.pinned} /></button>
        {supportsAgentCapability(r.tool, "delete") && <button className="row-act-btn row-act-danger" onClick={act(onRowDelete, r.key)}
          title={t("app:ctx.deleteSession")}>
          <TrashIcon size={13} /></button>}
        <button className="row-act-btn" onClick={act(onRowMore, r.key)}
          title={t("app:ctx.more")}><MoreDots /></button>
      </span>
    </div>
  );
});

// 会话库分组列表
export function LibraryList({ groups, collapsed, onToggle, empty, filtered, query, scanError,
  onClear, onRescan, onFullTextSearch,
  selectedId, multiSel,
  renamingKey, onRowClick, onRowPin, onRowDelete, onRowMore,
  onRowRename, onRowRenameSubmit, onRowRenameCancel }) {
  const { t } = useTranslation();
  if (empty) {
    // 扫描失败优先说:列表空是故障的后果,不是筛选或首次启动
    if (scanError) {
      return <PaneEmpty text={t("common:empty.scanFailed")} hint={t("common:empty.scanFailedHint")}
        detail={scanError}
        actions={[onRescan && { label: t("common:empty.retryScan"), onClick: onRescan, primary: true }]} />;
    }
    if (!filtered) {
      return <PaneEmpty text={t("common:empty.libraryNone")} hint={t("common:empty.libraryNoneHint")}
        actions={[onRescan && { label: t("common:empty.rescan"), onClick: onRescan }]} />;
    }
    // 侧栏只按标题匹配,全文检索在 ⌘K 面板里。搜不到的人不该靠猜才知道还有另一条路。
    return <PaneEmpty text={t("common:empty.library")}
      hint={query ? t("common:empty.titleOnlyHint") : null}
      actions={[
        query && onFullTextSearch
          && { label: t("common:empty.fullTextSearch"), onClick: onFullTextSearch, primary: true },
        { label: t("common:empty.clearFilter"), onClick: onClear },
      ]} />;
  }
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
      items.push({ key: r.key, y, h: ROW_H, node: (
        <LibraryRow r={r} selected={r.key === selectedId} multi={multiSet.has(r.key)}
          editing={r.key === renamingKey}
          onRowClick={onRowClick} onRowPin={onRowPin}
          onRowDelete={onRowDelete} onRowMore={onRowMore}
          onRowRename={onRowRename} onRowRenameSubmit={onRowRenameSubmit}
          onRowRenameCancel={onRowRenameCancel} />
      ) });
      y += ROW_H;
    });
    y += 3;
  });
  return <VirtualItems items={items} total={y} focusKey={selectedId} />;
}
// 迁移历史分组列表
export function HistoryList({ groups, empty, filtered, onClear, onDelete }) {
  const { t } = useTranslation();
  if (empty) {
    return filtered
      ? <PaneEmpty text={t("common:empty.history")}
          actions={[{ label: t("common:empty.clearFilter"), onClick: onClear }]} />
      : <PaneEmpty text={t("common:empty.historyNone")} hint={t("common:empty.historyNoneHint")} />;
  }
  const items = [];
  let y = 0;
  let focusKey = null;
  groups.forEach(g => {
    items.push({ key: `h:${g.label}`, y, h: HEADER_H, node: (
      <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "0 8px", height: HEADER_H }}>
        <span style={{ fontSize: 11, fontWeight: 600, color: "var(--tx3)" }}>{g.label}</span>
        <span style={{ fontSize: 11, color: "var(--tx5)" }}>· {g.rows.length}</span>
      </div>
    ) });
    y += HEADER_H;
    g.rows.forEach(h => {
      if (h.selected) focusKey = h.id;
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
  return <VirtualItems items={items} total={y} focusKey={focusKey} />;
}
