import Overview from "../modules/overview/Overview.jsx";
import SessionDetail from "../modules/browser/SessionDetail.jsx";
import HistoryDetail from "../modules/migration/HistoryDetail.jsx";
import FirstRun from "../modules/onboarding/FirstRun.jsx";
import AskFerry from "../modules/askferry/AskFerry.jsx";

export function WorkspaceRouter({
  view,
  sessions,
  historyRows,
  pricing,
  scanning,
  navigationTarget,
  currentSession,
  selectedSessionId,
  detailMeta,
  detail,
  detailActions,
  scope,
  ops,
  dirtyOps,
  applying,
  historySelection,
  ferry,
  agentAttachments,
  onAgentAttachmentsChange,
  onNavigate,
  onOpenConfig,
  environment,
  scan,
  onFirstDone,
  scanningLabel,
  emptyLibraryLabel,
}) {
  return (
    <>
      {view === "overview" && (
        <Overview sessions={sessions} historyRows={historyRows}
          prices={pricing?.prices || {}} scanning={scanning}
          navigationTarget={navigationTarget} />)}
      {view === "library" && (currentSession ? (
        <SessionDetail key={selectedSessionId}
          meta={detailMeta}
          data={detail?.data} error={detail?.error}
          onDiscardAll={detailActions.onDiscardAll}
          scope={scope} setScope={detailActions.setScope}
          ops={ops} dirtyOps={dirtyOps} addOp={detailActions.addOp} removeOp={detailActions.removeOp}
          updateOp={detailActions.updateOp}
          startReplyEdit={detailActions.startReplyEdit} replyEditError={detailActions.replyEditError}
          onOpenDiff={detailActions.onOpenDiff} onApply={detailActions.onApply} applying={applying}
          onOpenMigrate={detailActions.onOpenMigrate}
          navigationTarget={navigationTarget}
          onRefresh={detailActions.onRefresh} refreshing={detailActions.refreshing}
          onResume={detailActions.onResume} />
      ) : (
        <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
          color: "var(--tx5)", fontSize: 13 }}>
          {scanning ? scanningLabel : emptyLibraryLabel}
        </div>
      ))}
      {view === "history" && (
        <HistoryDetail h={historySelection} onDelete={detailActions.onDeleteHistory} />)}
      {view === "askferry" && (
        <AskFerry ferry={ferry} scanSessions={sessions}
          attachments={agentAttachments} onAttachmentsChange={onAgentAttachmentsChange}
          onNavigate={onNavigate}
          onOpenConfig={onOpenConfig} />)}
      {view === "firstrun" && <FirstRun env={environment} scan={scan} onStart={onFirstDone} />}
    </>
  );
}
