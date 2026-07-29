import { useMemo } from "react";

import { sessionIdentity } from "../modules/browser/public.js";
import { useAppChrome } from "../shared/capabilities/appChrome.jsx";
import { useBrowserState } from "../shared/capabilities/browserState.jsx";
import { useFerryRuntime } from "../shared/capabilities/ferryRuntime.jsx";
import { useOperationsState } from "../shared/capabilities/operationsState.jsx";
import { AppOverlays } from "./AppOverlays.jsx";
import { useSessionContentSearch } from "./useSessionContentSearch.js";

export function AppOverlayController({ t }) {
  const ferry = useFerryRuntime();
  const { toast, railTip, settings, guide } = useAppChrome();
  const { peek, search, contextMenu, deletion, tags, filters } =
    useBrowserState();
  const { migration, editing, floatChat } = useOperationsState();
  const isLibrarySearch = search.view !== "askferry" && search.view !== "history";
  // 全文命中只在 library 视图追加;engine 给的是 ref,要换回列表用的 identity key
  const contentSearch = useSessionContentSearch(
    search.pane?.query,
    Boolean(search.open && isLibrarySearch),
  );
  const identityByRef = useMemo(
    () =>
      new Map(
        (search.scanSessions || [])
          .filter((session) => session.ref)
          .map((session) => [session.ref, sessionIdentity(session)]),
      ),
    [search.scanSessions],
  );

  const filteredResults = (
    search.view === "askferry"
      ? search.ferrySessions.map((session) => ({
          id: session.session_id,
          title: session.title || t("askferry:chat.untitled"),
          tool: null,
          meta: session.model_id,
          onClick: () => ferry.openSession(session.session_id),
        }))
      : search.view === "history"
        ? search.historyGroups
            .flatMap((group) => group.rows)
            .map((item) => ({
              id: item.id,
              title: item.title,
              tool: item.tool,
              meta: `${item.from} → ${item.to}`,
              onClick: () => search.selectHistory(item.id),
            }))
        : search.libraryGroups
            .flatMap((group) => group.rows)
            .map((row) => ({
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

  // 标题已经命中的会话不重复出现在「全文匹配」里
  const alreadyListed = new Set(filteredResults.map((result) => result.id));
  const contentResults = isLibrarySearch
    ? (contentSearch?.sessions || []).flatMap((hit) => {
        const key = identityByRef.get(hit.ref);
        if (!key || alreadyListed.has(key)) return [];
        alreadyListed.add(key);
        return [
          {
            id: key,
            section: t("app:search.fullText"),
            title: hit.title || hit.project || hit.ref,
            tool: hit.tool,
            badge: hit.snippet ? t("app:search.contentHit") : null,
            meta: hit.snippet || hit.project,
            onClick: () => {
              search.setMultiSelection([]);
              search.selectSession(key);
            },
          },
        ];
      })
    : [];
  const searchResults = [...filteredResults, ...contentResults];
  // 索引还在建的时候「没结果」是误导:明说一句,别让用户以为真的搜不到
  const searchNotice =
    isLibrarySearch && contentSearch?.content_index?.ready === false
      ? t("app:search.indexing")
      : null;

  return (
    <AppOverlays
      t={t}
      floatChat={floatChat}
      peek={{
        open: Boolean(peek.id && peek.current),
        selectedId: peek.selectedId,
        meta: peek.meta,
        detail: peek.detail,
        actions: peek.actions,
        navigationTarget: peek.navigationTarget,
        refreshing: peek.refreshing,
        loadingMore: peek.loadingMore,
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
        notice: searchNotice,
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
        prepared: deletion.sessionConfirmation,
        onCancel: deletion.cancelSessionDeletion,
        onConfirm: deletion.confirmSessionDeletion,
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
        prepared: deletion.batchConfirmation,
        onCancel: deletion.cancelBatchDeletion,
        onConfirm: deletion.confirmBatchDeletion,
      }}
      tags={{
        selection: tags.selection,
        initial:
          tags.selection && !tags.selection.batch
            ? (tags.metaFor(tags.selection.sessions[0]).tags || []).join(", ")
            : "",
        onCancel: () => tags.setSelection(null),
        onConfirm: async (value) => {
          const selection = tags.selection;
          tags.setSelection(null);
          const nextTags = value
            .split(/[,，]/)
            .map((tag) => tag.trim())
            .filter(Boolean);
          for (const session of selection.sessions) {
            const merged = selection.batch
              ? [
                  ...new Set([
                    ...(tags.metaFor(session).tags || []),
                    ...nextTags,
                  ]),
                ]
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
        tools: filters.history.tools,
        onChange: filters.history.onChange,
        anchor: filters.anchor,
        onClose: filters.onClose,
        onClear: filters.history.onClear,
      }}
      guide={guide}
    />
  );
}
