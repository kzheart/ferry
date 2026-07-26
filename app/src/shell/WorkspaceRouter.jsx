import { useMemo } from "react";

import { Overview } from "../modules/overview/public.js";
import { SessionDetail } from "../modules/browser/public.js";
import { HistoryDetail } from "../modules/migration/public.js";
import { FirstRun } from "../modules/onboarding/public.js";
import { AskFerry } from "../modules/askferry/public.js";
import { useAppChrome } from "../shared/capabilities/appChrome.jsx";
import { useBrowserState } from "../shared/capabilities/browserState.jsx";
import { useOperationsState } from "../shared/capabilities/operationsState.jsx";

// 只保留各工作区独有、不属于任何领域 Context 的入参;会话、详情、扫描结果
// 这些跨工作区共享的状态一律从 Context 取。
export function WorkspaceRouter({
  historyRows,
  pricing,
  historySelection,
  agentAttachments,
  onAgentAttachmentsChange,
  onFirstDone,
  scanningLabel,
  emptyLibraryLabel,
}) {
  const { peek, search, deletion } = useBrowserState();
  const { floatChat } = useOperationsState();
  const { settings } = useAppChrome();

  const view = search.view;
  const sessions = search.scanSessions;
  const detailActions = useMemo(
    () => ({
      ...peek.actions,
      refreshing: peek.refreshing,
      loadingMore: peek.loadingMore,
      onDeleteHistory: deletion.onDeleteHistory,
    }),
    [peek.actions, peek.refreshing, peek.loadingMore, deletion.onDeleteHistory],
  );

  return (
    <>
      {view === "overview" && (
        <Overview
          sessions={sessions}
          historyRows={historyRows}
          prices={pricing?.prices || {}}
          scanning={settings.scanning}
          navigationTarget={peek.navigationTarget}
        />
      )}
      {view === "library" &&
        (peek.current ? (
          <SessionDetail
            key={peek.selectedId}
            meta={peek.meta}
            data={peek.detail?.data}
            error={peek.detail?.error}
            onOpenMigrate={detailActions.onOpenMigrate}
            navigationTarget={peek.navigationTarget}
            onRefresh={detailActions.onRefresh}
            refreshing={detailActions.refreshing}
            onLoadMore={detailActions.onLoadMore}
            loadingMore={detailActions.loadingMore}
            onResume={detailActions.onResume}
          />
        ) : (
          <div
            style={{
              flex: 1,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "var(--tx5)",
              fontSize: 13,
            }}
          >
            {settings.scanning ? scanningLabel : emptyLibraryLabel}
          </div>
        ))}
      {view === "history" && (
        <HistoryDetail
          h={historySelection}
          onDelete={detailActions.onDeleteHistory}
        />
      )}
      {view === "askferry" && (
        <AskFerry
          scanSessions={sessions}
          attachments={agentAttachments}
          onAttachmentsChange={onAgentAttachmentsChange}
          onNavigate={floatChat.onNavigate}
          onOpenConfig={floatChat.onOpenConfig}
        />
      )}
      {view === "firstrun" && (
        <FirstRun
          env={settings.env}
          scan={settings.scanResult}
          onStart={onFirstDone}
        />
      )}
    </>
  );
}
