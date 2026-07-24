import { Spinner } from "../components/ui/icons.jsx";
import AgentSessionList from "../modules/askferry/AgentSessionList.jsx";
import { HistoryList, LibraryList, Pane } from "./ResourcePane.jsx";

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
  return (
    <Pane collapsed={collapsed} width={width} dragging={resizing}
      title={pane.title} count={pane.count} placeholder={pane.placeholder}
      query={pane.query}
      onOpenSearch={onOpenSearch}
      onClearSearch={() => pane.onQuery({ target: { value: "" } })}
      filterCount={pane.filterCount}
      filterOn={filterOpen || pane.filterCount > 0}
      onFilter={onFilter}
      footer={pane.footer} tokens={pane.tokens}
      listKey={view}>
      {view === "library" && (
        library.scanning && !library.sessions.length
          ? <div style={{ padding: "34px 12px", textAlign: "center", color: "var(--tx5)",
              fontSize: 12, display: "flex", alignItems: "center", justifyContent: "center",
              gap: 8 }}><Spinner /> {library.scanningLabel}</div>
          : <LibraryList groups={library.groups}
              collapsed={library.collapsedGroups} onToggle={library.onToggleGroup}
              empty={library.groups.length === 0} onClear={library.onClear}
              selectedId={library.selectedId} multiSel={library.multiSel}
              onRowClick={library.onRowClick} onRowPin={library.onRowPin}
              onRowDelete={library.onRowDelete} onRowMore={library.onRowMore} />)}
      {view === "history" && (
        <HistoryList groups={history.groups} empty={history.filtered.length === 0}
          onDelete={history.onDelete}
          onClear={history.onClear} />)}
      {view === "askferry" && (
        <AgentSessionList sessions={agent.sessions}
          activeId={agent.activeId} onOpen={agent.onOpen} onNew={agent.onNew}
          onPin={agent.onPin} onDelete={agent.onDelete}
          onRename={agent.onRename} />)}
    </Pane>
  );
}
