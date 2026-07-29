import { Spinner } from "../shared/ui/icons.jsx";
import { AgentSessionList } from "../modules/askferry/public.js";
import { HistoryList, LibraryList, Pane, StaleScanNotice } from "./ResourcePane.jsx";

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
  // 空列表到底是筛出来的还是本来就空,决定空态说什么、给什么按钮
  const filtered = Boolean(pane.query) || pane.filterCount > 0;
  return (
    <Pane collapsed={collapsed} width={width} dragging={resizing}
      title={pane.title} count={pane.count}
      query={pane.query}
      onOpenSearch={onOpenSearch}
      onClearSearch={() => pane.onQuery({ target: { value: "" } })}
      filterCount={pane.filterCount}
      filterOn={filterOpen || pane.filterCount > 0}
      onFilter={onFilter}
      tokens={pane.tokens}
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
  );
}
