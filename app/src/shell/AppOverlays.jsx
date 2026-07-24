import { repoOf } from "../modules/browser/sessionModel.js";
import {
  BatchDeleteConfirm,
  LibraryFilter,
  SessionDeleteConfirm,
} from "../modules/browser/BrowserOverlays.jsx";
import { SessionPeekSheet } from "../modules/browser/SessionPeekSheet.jsx";
import { ApplyConfirm, DiffSheet } from "../modules/editing/EditOverlays.jsx";
import {
  HistoryDeleteConfirm,
  HistoryFilter,
} from "../modules/migration/HistoryOverlays.jsx";
import MigrateSheet from "../modules/migration/MigrateSheet.jsx";
import OrganizationPanel from "../modules/organizing/OrganizationPanel.jsx";
import { Guide } from "../modules/onboarding/Guide.jsx";
import SettingsPage from "../modules/settings/Settings.jsx";
import {
  ContextMenu,
  PromptBox,
  SearchPalette,
  Toast,
} from "../shared/ui/Overlays.jsx";

export function AppOverlays({
  t,
  organization,
  peek,
  migration,
  editing,
  search,
  contextMenu,
  sessionDelete,
  historyDelete,
  batchDelete,
  rename,
  agentRename,
  tags,
  toast,
  railTip,
  settings,
  libraryFilter,
  historyFilter,
  guide,
}) {
  return (
    <>
      {organization.open && (
        <OrganizationPanel
          sessions={organization.sessions.map(session => ({
            ...session,
            project: repoOf(session.dir),
          }))}
          onClose={organization.onClose}
          onApplied={organization.onApplied}
        />
      )}
      {peek.open && (
        <SessionPeekSheet
          selectedId={peek.selectedId}
          meta={peek.meta}
          detail={peek.detail}
          actions={peek.actions}
          scope={peek.scope}
          ops={peek.ops}
          dirtyOps={peek.dirtyOps}
          applying={peek.applying}
          navigationTarget={peek.navigationTarget}
          refreshing={peek.refreshing}
          onClose={peek.onClose}
          onOpenLibrary={peek.onOpenLibrary}
        />
      )}
      {migration.open && (
        <MigrateSheet
          meta={migration.meta}
          scope={migration.scope}
          env={migration.env}
          defaultProbe={migration.defaultProbe}
          terminalApp={migration.terminalApp}
          onClose={migration.onClose}
          onDone={migration.onDone}
        />
      )}
      {editing.diff && (
        <DiffSheet
          ops={editing.dirtyOps}
          preview={editing.diff.preview}
          loading={editing.diff.loading}
          error={editing.diff.error}
          onClose={editing.onCloseDiff}
        />
      )}
      {editing.confirmApply && (
        <ApplyConfirm
          ops={editing.dirtyOps}
          onCancel={editing.onCancelApply}
          onConfirm={editing.onConfirmApply}
        />
      )}
      {search.open && search.pane && (
        <SearchPalette
          placeholder={search.pane.placeholder}
          query={search.pane.query}
          onQuery={search.pane.onQuery}
          recentLabel={search.pane.query ? null : t("app:search.recent")}
          emptyLabel={t("app:search.empty")}
          results={search.results}
          onClose={search.onClose}
        />
      )}
      {contextMenu.open && contextMenu.items && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          items={contextMenu.items}
          onClose={contextMenu.onClose}
        />
      )}
      {sessionDelete.session && (
        <SessionDeleteConfirm
          sess={sessionDelete.session}
          onCancel={sessionDelete.onCancel}
          onConfirm={sessionDelete.onConfirm}
        />
      )}
      {historyDelete.history && (
        <HistoryDeleteConfirm
          history={historyDelete.history}
          onCancel={historyDelete.onCancel}
          onConfirm={historyDelete.onConfirm}
        />
      )}
      {batchDelete.sessions && (
        <BatchDeleteConfirm
          sessions={batchDelete.sessions}
          onCancel={batchDelete.onCancel}
          onConfirm={batchDelete.onConfirm}
        />
      )}
      {rename.session && (
        <PromptBox
          title={t("app:prompt.renameTitle")}
          desc={t("app:prompt.renameDesc", {
            title: rename.session.title || rename.session.id,
          })}
          placeholder={t("app:prompt.renamePlaceholder")}
          confirmLabel={t("app:prompt.save")}
          initial={rename.initial}
          onCancel={rename.onCancel}
          onConfirm={rename.onConfirm}
        />
      )}
      {agentRename.session && (
        <PromptBox
          title={t("askferry:pane.renameTitle")}
          desc={t("askferry:pane.renameDesc", {
            title: agentRename.session.title || t("askferry:chat.untitled"),
          })}
          placeholder={t("askferry:pane.renamePlaceholder")}
          confirmLabel={t("askferry:pane.save")}
          initial={agentRename.session.title || ""}
          onCancel={agentRename.onCancel}
          onConfirm={agentRename.onConfirm}
        />
      )}
      {tags.selection && (
        <PromptBox
          title={tags.selection.batch
            ? t("app:prompt.tagsBatchTitle", { n: tags.selection.sessions.length })
            : t("app:prompt.tagsTitle")}
          desc={tags.selection.batch
            ? t("app:prompt.tagsBatchDesc")
            : t("app:prompt.tagsDesc")}
          placeholder={t("app:prompt.tagsPlaceholder")}
          confirmLabel={t("app:prompt.save")}
          initial={tags.initial}
          onCancel={tags.onCancel}
          onConfirm={tags.onConfirm}
        />
      )}
      {toast.value && <Toast toast={toast.value} onDismiss={toast.onDismiss} />}
      {railTip.value && (
        <div
          style={{
            position: "absolute",
            left: railTip.railOnly ? 86 : 62,
            top: railTip.value.top,
            transform: "translateY(-50%)",
            zIndex: 60,
            background: "var(--tooltip)",
            color: "#fff",
            fontSize: 11,
            padding: "5px 9px",
            borderRadius: 6,
            boxShadow: "var(--shadow-menu)",
            pointerEvents: "none",
            whiteSpace: "nowrap",
            animation: "ffade .1s ease",
          }}
        >
          {railTip.value.label}
        </div>
      )}
      {settings.open && (
        <SettingsPage
          settings={settings.value}
          setSettings={settings.onChange}
          updater={settings.updater}
          ferry={settings.ferry}
          initialSection={settings.initialSection}
          scan={settings.scan}
          env={settings.env}
          scanning={settings.scanning}
          onRescan={settings.onRescan}
          guideSeen={settings.guideSeen}
          onOpenGuide={settings.onOpenGuide}
          onFirstRun={settings.onFirstRun}
          onClose={settings.onClose}
        />
      )}
      {libraryFilter.open && (
        <LibraryFilter
          f={libraryFilter.value}
          setF={libraryFilter.onChange}
          counts={libraryFilter.counts}
          dirs={libraryFilter.dirs}
          tags={libraryFilter.tags}
          anchor={libraryFilter.anchor}
          onClose={libraryFilter.onClose}
          onClear={libraryFilter.onClear}
        />
      )}
      {historyFilter.open && (
        <HistoryFilter
          f={historyFilter.value}
          setF={historyFilter.onChange}
          anchor={historyFilter.anchor}
          onClose={historyFilter.onClose}
          onClear={historyFilter.onClear}
        />
      )}
      {guide.step > 0 && (
        <Guide
          step={guide.step}
          onGo={guide.onGo}
          onFinish={guide.onFinish}
        />
      )}
    </>
  );
}
