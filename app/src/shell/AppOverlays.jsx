import {
  BatchDeleteConfirm,
  SessionDeleteConfirm,
  SessionPeekSheet,
} from "../modules/browser/public.js";
import { ApplyConfirm, DiffSheet } from "../modules/editing/public.js";
import { MigrateSheet } from "../modules/migration/public.js";
import { FloatingAgentPanel } from "../modules/askferry/public.js";
import { Guide } from "../modules/onboarding/public.js";
import { SettingsPage, UpdateAnnouncement } from "../modules/settings/public.js";
import { ContextMenu } from "../shared/ui/ContextMenu.jsx";
import { PromptBox } from "../shared/ui/PromptBox.jsx";
import { Toast } from "../shared/ui/Toast.jsx";
import { SearchPalette } from "./SearchPalette.jsx";

export function AppOverlays({
  t,
  floatChat,
  peek,
  migration,
  editing,
  search,
  contextMenu,
  sessionDelete,
  batchDelete,
  tags,
  toast,
  settings,
  guide,
  updateAnnouncement = { value: null },
}) {
  return (
    <>
      {floatChat.mounted && (
        <FloatingAgentPanel
          open={floatChat.open}
          onToggle={floatChat.onToggle}
          session={floatChat.session}
          scanSessions={floatChat.scanSessions}
          onNavigate={floatChat.onNavigate}
          onOpenConfig={floatChat.onOpenConfig}
          onOpenFull={floatChat.onOpenFull}
        />
      )}
      {peek.open && (
        <SessionPeekSheet
          selectedId={peek.selectedId}
          meta={peek.meta}
          detail={peek.detail}
          actions={peek.actions}
          navigationTarget={peek.navigationTarget}
          loadingMore={peek.loadingMore}
          onClose={peek.onClose}
          onOpenLibrary={peek.onOpenLibrary}
        />
      )}
      {migration.open && (
        <MigrateSheet
          meta={migration.meta}
          scope={migration.scope}
          env={migration.env}
          terminalApp={migration.terminalApp}
          onClose={migration.onClose}
          onDone={migration.onDone}
          onResumeElsewhere={migration.onResumeElsewhere}
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
          notice={search.notice}
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
      {sessionDelete.prepared && (
        <SessionDeleteConfirm
          prepared={sessionDelete.prepared}
          onCancel={sessionDelete.onCancel}
          onConfirm={sessionDelete.onConfirm}
        />
      )}
      {batchDelete.prepared && (
        <BatchDeleteConfirm
          prepared={batchDelete.prepared}
          onCancel={batchDelete.onCancel}
          onConfirm={batchDelete.onConfirm}
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
      {settings.open && (
        <SettingsPage
          settings={settings.value}
          setSettings={settings.onChange}
          updater={settings.updater}
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
      {guide.step > 0 && (
        <Guide
          step={guide.step}
          steps={guide.steps}
          onGo={guide.onGo}
          onFinish={guide.onFinish}
        />
      )}
      {/* 更新后的第一屏,压在所有弹层之上:它是本次启动最该先被读到的东西 */}
      {updateAnnouncement.value && (
        <UpdateAnnouncement
          announcement={updateAnnouncement.value}
          onDismiss={updateAnnouncement.onDismiss}
          onOpenUpdates={updateAnnouncement.onOpenUpdates}
        />
      )}
    </>
  );
}
