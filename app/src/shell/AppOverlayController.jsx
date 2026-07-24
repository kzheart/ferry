import { AppOverlays } from "./AppOverlays.jsx";

export function AppOverlayController({
  t,
  organization,
  peek,
  migration,
  editing,
  search,
  contextMenu,
  deletion,
  rename,
  agentRename,
  tags,
  toast,
  railTip,
  settings,
  filters,
  guide,
}) {
  const searchResults = (search.view === "askferry"
    ? search.ferrySessions.map(session => ({
        id: session.session_id,
        title: session.title || t("askferry:chat.untitled"),
        tool: null,
        meta: session.model_id,
        onClick: () => search.ferry.openSession(session.session_id),
      }))
    : search.view === "history"
      ? search.historyGroups.flatMap(group => group.rows).map(item => ({
          id: item.id,
          title: item.title,
          tool: item.tool,
          meta: `${item.from} → ${item.to}`,
          onClick: () => search.selectHistory(item.id),
        }))
      : search.libraryGroups.flatMap(group => group.rows).map(row => ({
          id: row.key,
          title: row.title,
          tool: row.tool,
          meta: row.repo,
          onClick: () => {
            search.setMultiSelection([]);
            search.selectSession(row.key);
          },
        }))
  ).slice(0, 60);

  return (
    <AppOverlays
      t={t}
      organization={{
        open: organization.open,
        sessions: organization.sessions,
        onClose: () => organization.setOpen(false),
        onApplied: () => {
          organization.reloadMetadata();
          organization.scan();
        },
      }}
      peek={{
        open: Boolean(peek.id && peek.current),
        selectedId: peek.selectedId,
        meta: peek.meta,
        detail: peek.detail,
        actions: peek.actions,
        scope: peek.scope,
        ops: peek.ops,
        dirtyOps: peek.dirtyOps,
        applying: peek.applying,
        navigationTarget: peek.navigationTarget,
        refreshing: peek.refreshing,
        onClose: () => peek.setId(null),
        onOpenLibrary: () => {
          peek.setId(null);
          peek.setView("library");
        },
      }}
      migration={{
        open: Boolean(migration.state && migration.current),
        meta: migration.current,
        scope: migration.state?.scope,
        env: migration.env,
        defaultProbe: Boolean(migration.settings.runtimeProbe),
        terminalApp: migration.settings.terminalApp,
        onClose: () => migration.setState(null),
        onDone: migration.loadHistory,
      }}
      editing={{
        diff: editing.diff,
        dirtyOps: editing.dirtyOps,
        confirmApply: editing.confirmApply,
        onCloseDiff: () => editing.setDiff(null),
        onCancelApply: () => editing.setConfirmApply(false),
        onConfirmApply: editing.apply,
      }}
      search={{
        open: search.open,
        pane: search.pane,
        results: searchResults,
        onClose: () => search.setOpen(false),
      }}
      contextMenu={{
        open: Boolean(contextMenu.value),
        x: contextMenu.value?.x,
        y: contextMenu.value?.y,
        items: contextMenu.items,
        onClose: () => contextMenu.setValue(null),
      }}
      sessionDelete={{
        session: deletion.session,
        onCancel: () => deletion.setSession(null),
        onConfirm: () => {
          const session = deletion.session;
          deletion.setSession(null);
          deletion.deleteSession(session);
        },
      }}
      historyDelete={{
        history: deletion.history,
        onCancel: () => deletion.setHistory(null),
        onConfirm: () => {
          if (deletion.history._id === deletion.selectedHistoryId) {
            deletion.selectHistory(null);
          }
          const id = deletion.history.id;
          deletion.setHistory(null);
          deletion.deleteHistory(id).catch(() => {});
        },
      }}
      batchDelete={{
        sessions: deletion.batch,
        onCancel: () => deletion.setBatch(null),
        onConfirm: deletion.deleteBatch,
      }}
      rename={{
        session: rename.session,
        initial: rename.session
          ? rename.metaFor(rename.session).name || rename.session.title || ""
          : "",
        onCancel: () => rename.setSession(null),
        onConfirm: value => {
          const session = rename.session;
          rename.setSession(null);
          rename.updateMetadata(session, { name: value });
        },
      }}
      agentRename={{
        session: agentRename.session,
        onCancel: () => agentRename.setSession(null),
        onConfirm: title => {
          const session = agentRename.session;
          agentRename.setSession(null);
          if (title) {
            agentRename.ferry.rename(session.session_id, title)
              .catch(agentRename.ferry.reportError);
          }
        },
      }}
      tags={{
        selection: tags.selection,
        initial: tags.selection && !tags.selection.batch
          ? (tags.metaFor(tags.selection.sessions[0]).tags || []).join(", ")
          : "",
        onCancel: () => tags.setSelection(null),
        onConfirm: async value => {
          const selection = tags.selection;
          tags.setSelection(null);
          const nextTags = value.split(/[,，]/)
            .map(tag => tag.trim())
            .filter(Boolean);
          for (const session of selection.sessions) {
            const merged = selection.batch
              ? [...new Set([...(tags.metaFor(session).tags || []), ...nextTags])]
              : nextTags;
            await tags.updateMetadata(session, { tags: merged });
          }
        },
      }}
      toast={{ value: toast.value, onDismiss: () => toast.setValue(null) }}
      railTip={railTip}
      settings={{
        open: settings.open,
        value: settings.value,
        onChange: settings.onChange,
        updater: settings.updater,
        ferry: settings.ferry,
        initialSection: settings.section,
        scan: settings.scanResult,
        env: settings.env,
        scanning: settings.scanning,
        onRescan: settings.scan,
        guideSeen: settings.guideSeen,
        onOpenGuide: () => {
          settings.setOpen(false);
          settings.openGuide();
        },
        onFirstRun: () => {
          settings.setOpen(false);
          settings.setView("firstrun");
        },
        onClose: () => settings.setOpen(false),
      }}
      libraryFilter={{
        open: filters.popover === "lib",
        value: filters.library.value,
        onChange: filters.library.onChange,
        counts: filters.library.counts,
        dirs: filters.library.dirs,
        tags: filters.library.tags,
        anchor: filters.anchor,
        onClose: filters.onClose,
        onClear: filters.library.onClear,
      }}
      historyFilter={{
        open: filters.popover === "hist",
        value: filters.history.value,
        onChange: filters.history.onChange,
        anchor: filters.anchor,
        onClose: filters.onClose,
        onClear: filters.history.onClear,
      }}
      guide={guide}
    />
  );
}
