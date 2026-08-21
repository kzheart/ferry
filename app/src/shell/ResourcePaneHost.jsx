import { useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Spinner } from "../shared/ui/icons.jsx";
import { ContextMenu } from "../shared/ui/ContextMenu.jsx";
import { AgentSessionList } from "../modules/askferry/public.js";
import { HistoryList, LibraryList, Pane, StaleScanNotice } from "./ResourcePane.jsx";
import { DisplayMenu, ScopeMenu } from "./LibraryPaneMenus.jsx";

export function ResourcePaneHost({
  view,
  pane,
  collapsed,
  width,
  resizing,
  filterOpen,
  onOpenSearch,
  onFilter,
  library,
  history,
  agent,
}) {
  const { t } = useTranslation();
  // 空列表到底是筛出来的还是本来就空,决定空态说什么、给什么按钮
  const filtered = Boolean(pane.query) || pane.filterCount > 0;
  const isLibrary = view === "library";
  const inProjectScope = isLibrary && library.scope.kind === "project";
  const onlyProject = dir => library.onSelectScope({ kind: "project", value: dir });
  const titleRef = useRef(null);
  const displayRef = useRef(null);
  const [scopeOpen, setScopeOpen] = useState(false);
  const [displayOpen, setDisplayOpen] = useState(false);
  // 文件夹头的右键菜单:内容与悬停出现的两个动作一致,自成一体,不走会话行那套 ctxMenu
  const [folderMenu, setFolderMenu] = useState(null);
  // 导航栏展开时范围就在左边常驻,标题不必再当入口;折叠时它是唯一的入口
  const titleIsMenu = isLibrary && library.navCollapsed;

  return (
    <>
    <Pane collapsed={collapsed} width={width} dragging={resizing}
      title={pane.title} count={pane.count}
      query={pane.query}
      onQuery={pane.onQuery}
      searchInline={isLibrary}
      placeholder={pane.placeholder}
      onOpenSearch={onOpenSearch}
      onClearSearch={() => pane.onQuery({ target: { value: "" } })}
      onTitleClick={titleIsMenu
        ? event => {
            titleRef.current = event.currentTarget;
            setScopeOpen(value => !value);
          }
        : null}
      titleMenuOpen={scopeOpen}
      onBack={inProjectScope ? () => library.onSelectScope({ kind: "all" }) : null}
      backLabel={t("app:nav.backToAll")}
      displayLabel={pane.displayLabel}
      displayDot={pane.filterCount > 0}
      displayOn={isLibrary ? displayOpen : (filterOpen || pane.filterCount > 0)}
      onDisplay={event => {
        if (!isLibrary) { onFilter(event); return; }
        displayRef.current = event.currentTarget;
        setDisplayOpen(value => !value);
      }}
      listKey={view}>
      {view === "library" && (
        library.scanning && !library.sessions.length
          ? <div style={{ padding: "34px 12px", textAlign: "center", color: "var(--tx5)",
              fontSize: 12, display: "flex", alignItems: "center", justifyContent: "center",
              gap: 8 }}><Spinner /> {library.scanningLabel}</div>
          : <>
            {/* 扫描失败但还有上次的结果:列表照常可用,但必须说清眼前是旧数据 */}
            {library.scanError && library.groups.length > 0 && (
              <StaleScanNotice error={library.scanError} scanning={library.scanning}
                onRescan={library.onRescan} />
            )}
            <LibraryList groups={library.groups}
              groupMode={library.groupMode} scopeKind={library.scope.kind}
              favorites={library.favorites}
              onFavoriteProject={library.onFavoriteProject}
              onOnlyProject={onlyProject}
              onFolderMenu={(dir, event) =>
                setFolderMenu({ dir, x: event.clientX, y: event.clientY })}
              collapsed={library.collapsedGroups} onToggle={library.onToggleGroup}
              empty={library.groups.length === 0} filtered={filtered}
              query={pane.query} scanError={library.scanError}
              onClear={library.onClear} onRescan={library.onRescan}
              onFullTextSearch={onOpenSearch}
              selectedId={library.selectedId} multiSel={library.multiSel}
              renamingKey={library.renamingKey}
              onRowClick={library.onRowClick} onRowPin={library.onRowPin}
              onRowDelete={library.onRowDelete} onRowMore={library.onRowMore}
              onRowRename={library.onRowRename}
              onRowRenameSubmit={library.onRowRenameSubmit}
              onRowRenameCancel={library.onRowRenameCancel} />
          </>)}
      {view === "history" && (
        <HistoryList groups={history.groups} empty={history.filtered.length === 0}
          filtered={filtered} onDelete={history.onDelete}
          onClear={history.onClear} />)}
      {view === "askferry" && (
        <AgentSessionList sessions={agent.sessions} />)}
    </Pane>
    {isLibrary && scopeOpen && titleRef.current && (
      <ScopeMenu anchorRef={titleRef} scope={library.scope}
        scopeCounts={library.scopeCounts} projects={library.projects}
        favorites={library.favorites}
        toolNames={library.toolNames} onPick={library.onSelectScope}
        onClose={() => setScopeOpen(false)} />
    )}
    {isLibrary && displayOpen && displayRef.current && (
      <DisplayMenu anchorRef={displayRef} display={library.display}
        onChange={library.onDisplayChange} onClose={() => setDisplayOpen(false)} />
    )}
    {folderMenu && createPortal(
      <ContextMenu x={folderMenu.x} y={folderMenu.y} onClose={() => setFolderMenu(null)}
        items={[
          { label: t(library.favorites?.includes(folderMenu.dir)
              ? "app:ctx.unfavoriteProject" : "app:ctx.favoriteProject"),
            onClick: () => library.onFavoriteProject(folderMenu.dir) },
          { label: t("app:ctx.onlyThisProject"),
            onClick: () => onlyProject(folderMenu.dir) },
        ]} />,
      document.body)}
    </>
  );
}
