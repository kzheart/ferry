// 上下文资源栏:三种视图共享同一骨架(标题+搜索/筛选图标/标签/列表/页脚)
import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ACCENT } from "../shared/ui/toolDisplay.js";
import { TOOL_NAME } from "../shared/contracts/tools.js";
import { libraryGroupExpanded } from "../modules/browser/public.js";
import { ArrowRightIcon, BranchIcon, Caret, ChevronLeftIcon, CloseIcon, FilterIcon, MoreDots, PinIcon,
  SearchIcon, StarIcon, ToolIcon } from "../shared/ui/icons.jsx";
import { useDensityMetrics } from "../shared/ui/density.js";
import { TITLEBAR_INSET } from "../shared/ui/platform.js";

export function Pane({ collapsed, width, dragging, title, count,
  query, onQuery, searchInline, placeholder, onOpenSearch, onClearSearch,
  displayDot, displayOn, onDisplay, displayLabel,
  onBack, backLabel, listKey, headerExtra, children }) {
  const { t } = useTranslation();
  const searchRef = useRef(null);
  // ⌘F 聚焦常驻筛选框;标题行搜索按钮打开全文 / 命令面板
  useEffect(() => {
    if (!searchInline) return undefined;
    const focus = () => searchRef.current?.focus();
    document.addEventListener("ferry-focus-pane-search", focus);
    return () => document.removeEventListener("ferry-focus-pane-search", focus);
  }, [searchInline]);
  return (
    <div data-guide="pane"
      aria-hidden={collapsed || undefined}
      style={{ width: collapsed ? 0 : width, flex: "none", overflow: "hidden",
      background: "var(--pane)",
      borderRight: collapsed ? "none" : "1px solid var(--line)",
      transition: dragging ? "width 0s" : "width .2s ease-out" }}>
      <div style={{ width, height: "100%", display: "flex", flexDirection: "column",
        minHeight: 0, opacity: collapsed ? 0 : 1,
        visibility: collapsed ? "hidden" : "visible",
        transition: collapsed
          ? "opacity .12s ease, visibility 0s linear .2s"
          : "opacity .12s ease" }}>
        {/* 通高侧栏:顶部 44px 归红绿灯,整块可拖拽窗口 */}
        <div data-tauri-drag-region style={{ height: TITLEBAR_INSET, flex: "none" }} />
        <div style={{ flex: "none", padding: "0 10px 0" }}>
          {/* 标题行:范围名 + 计数,右侧搜索与显示选项两个图标按钮 */}
          <div style={{ display: "flex", alignItems: "center", gap: 6, height: 28 }}>
            {/* 进入某个项目范围之后要有一条回得去的路:标题左侧的返回箭头 */}
            {onBack && (
              <button type="button" className="row-act-btn" data-pane-back
                title={backLabel} aria-label={backLabel} onClick={onBack}
                style={{ flex: "none", width: 18, height: 18, marginLeft: -2 }}>
                <ChevronLeftIcon size={13} />
              </button>
            )}
            <span style={{ fontSize: "var(--fs-title)", fontWeight: 600, color: "var(--tx1)",
              letterSpacing: "-.01em", minWidth: 0, overflow: "hidden",
              textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{title}</span>
            <span className="mono tnum" style={{ fontSize: "var(--fs-meta)",
              color: "var(--tx5)" }}>{count}</span>
            <span style={{ flex: 1 }} />
            <button className="ftool-btn" data-guide="search"
              title={t("app:pane.search")} onClick={onOpenSearch}
              style={query ? { background: "var(--fill4)", color: "var(--tx1)" } : undefined}>
              <SearchIcon /></button>
            <button className="ftool-btn" data-guide="display"
              title={displayLabel} onClick={onDisplay}
              style={{ position: "relative",
                ...(displayOn ? { background: "var(--fill4)", color: "var(--tx1)" } : {}) }}>
              <FilterIcon />
              {displayDot && (
                <span style={{ position: "absolute", top: 3, right: 3, width: 6, height: 6,
                  borderRadius: "50%", background: ACCENT }} />
              )}</button>
          </div>
          {searchInline ? (
            <div className="lib-search" style={{ display: "flex", alignItems: "center", gap: 6,
              height: 26, padding: "0 7px", marginTop: 8, borderRadius: 7,
              background: "var(--fill4)", boxShadow: "inset 0 0 0 .5px var(--line)" }}>
              <SearchIcon />
              <input ref={searchRef} data-pane-search value={query} onChange={onQuery}
                placeholder={placeholder}
                onKeyDown={event => { if (event.key === "Escape") onClearSearch(); }}
                style={{ flex: 1, minWidth: 0, height: 22, border: "none", outline: "none",
                  background: "transparent", color: "var(--tx1)", fontSize: 12.5,
                  fontFamily: "inherit", padding: 0 }} />
              {query && (
                <button className="row-act-btn" onClick={onClearSearch}
                  title={t("common:empty.clearFilter")}><CloseIcon size={11} /></button>
              )}
            </div>
          ) : query ? (
            <div style={{ display: "flex", alignItems: "center", gap: 6, height: 26, padding: "0 6px 0 10px",
              background: "var(--acc-soft3)", border: "1px solid var(--acc-line)", borderRadius: 6,
              marginTop: 9, fontSize: 11, color: "var(--acc-text)" }}>
              <SearchIcon />
              <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis",
                whiteSpace: "nowrap" }}>{query}</span>
              <button className="row-act-btn" onClick={onClearSearch}
                title={t("common:empty.clearFilter")}><CloseIcon size={11} /></button>
            </div>
          ) : null}
          {headerExtra}
        </div>
        <div data-pane-scroll className="fscroll" tabIndex={-1}
          onKeyDown={event => {
            // 列表聚焦时 Esc 等于点了标题左边那个返回箭头
            if (event.key === "Escape" && onBack) { event.stopPropagation(); onBack(); }
          }}
          style={{ flex: 1, overflowY: "auto", padding: "8px 8px 10px", minHeight: 0,
            outline: "none" }}>
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
// 行高随密度变(shared/ui/density.js 维护数值),虚拟化要的是数字而不是 CSS 变量,
// 所以这里从密度表里取,并把它一路传进行内 style,两边必须是同一个值。
const ROW_ICON = 16;   // 会话行前的 agent 图标
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

const FolderGlyph = ({ size = 12 }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden
    style={{ flex: "none", color: "currentColor" }}>
    <path d="M2 4.2c0-.7.5-1.2 1.2-1.2h2.4l1.3 1.5h5.9c.7 0 1.2.5 1.2 1.2v5.6c0 .7-.5 1.2-1.2 1.2H3.2c-.7 0-1.2-.5-1.2-1.2V4.2z"
      stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" />
  </svg>
);

// 项目文件夹头:折叠箭头 + 文件夹图标 + 文件夹名 + 计数。
// 同名仓库不再靠淡色父路径消歧(分组键仍是完整 dir),完整路径放进 title。
// 悬停时右侧浮现两个动作(计数让位):☆ 收藏到导航栏、→ 只看此项目;右键同样两项。
function FolderRow({ group, expanded, height, favorite, onToggle, onFavorite, onOnly, onMenu }) {
  const { t } = useTranslation();
  const act = fn => event => { event.stopPropagation(); fn(group.dir); };
  return (
    <div className="hov-row folder-row" onClick={onToggle} title={group.dir}
      onContextMenu={onMenu
        ? event => { event.preventDefault(); event.stopPropagation(); onMenu(group.dir, event); }
        : undefined}
      style={{ display: "flex", alignItems: "center", gap: 6, padding: "0 8px", height,
        cursor: "default", borderRadius: 6 }}>
      <Caret open={expanded} />
      <span style={{ color: "var(--tx4)", display: "flex" }}><FolderGlyph /></span>
      <span style={{ fontSize: "var(--fs-meta)", color: "var(--tx2)", flex: 1, minWidth: 0,
        overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{group.label}</span>
      <span className="row-meta mono tnum" style={{ fontSize: "var(--fs-meta)",
        color: "var(--tx5)", flex: "none" }}>{group.count}</span>
      {onFavorite && (
        <span className="row-act" style={{ gap: 1, flex: "none" }}>
          <button className="row-act-btn" onClick={act(onFavorite)}
            title={t(favorite ? "app:ctx.unfavoriteProject" : "app:ctx.favoriteProject")}
            aria-label={t(favorite ? "app:ctx.unfavoriteProject" : "app:ctx.favoriteProject")}
            style={{ width: 20, height: 20, ...(favorite ? { color: "var(--warn)" } : {}) }}>
            <StarIcon size={13} filled={favorite} /></button>
          <button className="row-act-btn" onClick={act(onOnly)}
            title={t("app:ctx.onlyThisProject")} aria-label={t("app:ctx.onlyThisProject")}
            style={{ width: 20, height: 20 }}>
            <ArrowRightIcon size={13} /></button>
        </span>
      )}
    </div>
  );
}

// 会话行:双行 40px(标题 / 元信息),悬浮浮现操作按钮(置顶/更多);双击标题就地重命名
const LibraryRow = memo(function LibraryRow({ r, height, selected, multi, editing, showRepo,
  guide, onRowClick, onRowPin, onRowMore,
  onRowRename, onRowRenameSubmit, onRowRenameCancel }) {
  const { t } = useTranslation();
  const act = (fn, key) => e => { e.stopPropagation(); fn(key, e); };
  // Enter/Esc 已经了结这次重命名后,紧随的 blur 不能再触发一次提交
  const settled = useRef(false);

  if (editing) {
    return (
      <div className="lib-row lib-row-tall" style={{ display: "flex", gap: 8, alignItems: "center",
        padding: "0 8px", height, borderRadius: 6, background: "var(--acc-soft2)" }}>
        <ToolIcon tool={r.tool} size={ROW_ICON} />
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
          style={{ flex: 1, minWidth: 0, height: 24, border: "none", outline: "none",
            background: "transparent", color: "var(--tx1)", fontSize: 13, padding: 0 }} />
      </div>
    );
  }

  // 元信息:「仓库名 + 分支图标 + 分支」;选定项目范围时仓库名已在标题,只留分支。
  // 不写 Agent 名(左侧图标已说明)。条数默认隐去,悬停再淡入。
  const bits = [];
  if (showRepo && r.repo) {
    bits.push(
      <span key="repo" className="lib-meta-bit" title={r.dir}>{r.repo}</span>,
    );
  }
  if (r.branch) {
    bits.push(
      <span key="branch" className="lib-meta-bit" title={r.branch}>
        <BranchIcon size={12} />
        <span>{r.branch}</span>
      </span>,
    );
  }
  if (r.count != null) {
    bits.push(
      <span key="count" className="lib-meta-bit lib-count mono tnum">
        {t("app:library.metaCount", { n: r.count })}
      </span>,
    );
  }

  return (
    <div onClick={e => onRowClick(r.key, e)}
      onDoubleClick={e => { if (!e.target.closest(".row-act")) onRowRename(r.key); }}
      onContextMenu={e => { e.preventDefault(); e.stopPropagation(); onRowMore(r.key, e); }}
      title={r.dir}
      data-guide={guide ? "session-row" : undefined}
      className={selected || multi ? "lib-row lib-row-tall" : "lib-row lib-row-tall hov-item"}
      style={{ display: "flex", gap: 8, alignItems: "flex-start", padding: "5px 8px", height,
        borderRadius: 6, cursor: "default", transition: "background .12s ease",
        ...rowSel(selected || multi) }}>
      <span title={TOOL_NAME[r.tool] || r.tool} style={{ flex: "none", display: "flex" }}>
        <ToolIcon tool={r.tool} size={ROW_ICON} />
      </span>
      <span style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 1,
        marginTop: -3 }}>
        <span style={{ display: "flex", alignItems: "center", gap: 5 }}>
          <span style={{ fontSize: "var(--fs-body)", fontWeight: 500, color: "var(--tx1)", whiteSpace: "nowrap",
            overflow: "hidden", textOverflow: "ellipsis", flex: 1, minWidth: 0 }}>{r.title}</span>
          {r.pinned && <PinGlyph />}
          {r.hasSub && <span className="lib-count" title={r.subLabel}
            style={{ fontSize: 11, color: "var(--tx5)", flex: "none" }}>+{r.subCount}</span>}
          {r.hasMig && <span className="row-meta" title={t("app:library.hasMig")}
            style={{ width: 5, height: 5, borderRadius: "50%",
              background: "var(--info-dot)", flex: "none" }} />}
        </span>
        {bits.length > 0 && (
          <span className="lib-meta" style={{ fontSize: "var(--fs-meta)", color: "var(--tx5)",
            whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis",
            display: "flex", alignItems: "center", gap: 5, minWidth: 0 }}>
            {bits}
          </span>
        )}
      </span>
      <span className="row-meta" style={{ fontSize: 11, color: "var(--tx5)", flex: "none",
        marginTop: -1 }}>{r.active}</span>
      <span className="row-act" style={{ gap: 1, flex: "none" }}>
        <button className="row-act-btn" onClick={act(onRowPin, r.key)}
          title={r.pinned ? t("app:ctx.unpin") : t("app:ctx.pin")}
          style={r.pinned ? { color: ACCENT } : undefined}>
          <PinIcon filled={r.pinned} /></button>
        <button className="row-act-btn" onClick={act(onRowMore, r.key)}
          title={t("app:ctx.more")}><MoreDots /></button>
      </span>
    </div>
  );
});

// 会话库分组列表
export function LibraryList({ groups, collapsed, onToggle, empty, filtered, query, scanError,
  groupMode = "time", scopeKind = "all",
  favorites = [], onFavoriteProject, onOnlyProject, onFolderMenu,
  onClear, onRescan, onFullTextSearch,
  selectedId, multiSel,
  renamingKey, onRowClick, onRowPin, onRowMore,
  onRowRename, onRowRenameSubmit, onRowRenameCancel }) {
  const { t } = useTranslation();
  const metrics = useDensityMetrics();
  const ROW_H = metrics.libRow;
  const FOLDER_H = metrics.folderRow;
  const HEADER_H = metrics.groupHeader;
  const favoriteSet = new Set(favorites);
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
  // 项目视图里行已经归在文件夹下,选中项目范围后项目名也已在标题上,都不必再重复
  const showRepo = scopeKind !== "project" && groupMode !== "project";
  const items = [];
  let y = 0;
  // 引导「右键菜单」那步要有一行可高亮:优先选中行,没有选中就用列表里的第一行
  let guideKey = selectedId || null;
  groups.forEach(g => {
    const project = g.kind === "project";
    const expanded = libraryGroupExpanded(g, collapsed, query || "");
    // 不分组时整条列表只有会话行,没有任何分组头
    if (g.kind === "flat") {
      g.rows.forEach(r => {
        if (!guideKey) guideKey = r.key;
        items.push({ key: r.key, y, h: ROW_H, node: (
          <LibraryRow r={r} height={ROW_H} selected={r.key === selectedId} multi={multiSet.has(r.key)}
            editing={r.key === renamingKey} showRepo={showRepo}
            guide={r.key === guideKey} onRowClick={onRowClick} onRowPin={onRowPin}
            onRowMore={onRowMore}
            onRowRename={onRowRename} onRowRenameSubmit={onRowRenameSubmit}
            onRowRenameCancel={onRowRenameCancel} />
        ) });
        y += ROW_H;
      });
      return;
    }
    const headerH = project ? FOLDER_H : HEADER_H;
    items.push({ key: `h:${g.key}`, y, h: headerH, node: project ? (
      <FolderRow group={g} expanded={expanded} height={FOLDER_H}
        favorite={favoriteSet.has(g.dir)}
        onToggle={() => onToggle(g.key, g.kind)}
        onFavorite={onFavoriteProject} onOnly={onOnlyProject} onMenu={onFolderMenu} />
    ) : (
      <div className="hov-row group-header" onClick={() => onToggle(g.key, g.kind)}
        style={{ display: "flex", alignItems: "center", gap: 5, padding: "0 8px", height: HEADER_H,
          cursor: "default", borderRadius: 6 }}>
        <Caret open={expanded} />
        <span style={{ fontSize: 11, fontWeight: 600, color: "var(--tx3)" }}>{g.label}</span>
        <span className="group-count" style={{ fontSize: 11, color: "var(--tx5)" }}>· {g.count}</span>
      </div>
    ) });
    y += headerH;
    if (expanded) g.rows.forEach(r => {
      if (!guideKey) guideKey = r.key;
      items.push({ key: r.key, y, h: ROW_H, node: (
        <LibraryRow r={r} height={ROW_H} selected={r.key === selectedId} multi={multiSet.has(r.key)}
          editing={r.key === renamingKey} showRepo={showRepo}
          guide={r.key === guideKey} onRowClick={onRowClick} onRowPin={onRowPin}
          onRowMore={onRowMore}
          onRowRename={onRowRename} onRowRenameSubmit={onRowRenameSubmit}
          onRowRenameCancel={onRowRenameCancel} />
      ) });
      y += ROW_H;
    });
    y += 3;
  });
  return <VirtualItems items={items} total={y} focusKey={selectedId} />;
}
