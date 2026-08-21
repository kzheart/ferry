import { useEffect } from "react";

export function useAppKeyboardShortcuts({
  paneAvailable,
  onOpenSearch,
  onToggleNav,
  onFocusPaneSearch,
  dismissers,
  view,
  currentSession,
  multiIds,
  sessionsByKey,
  onRename,
  onBatchDelete,
  onDelete,
  onResume,
  libraryVisibleIds,
  historyVisibleIds,
  selectedSessionId,
  selectedHistoryId,
  selectSession,
  selectHistory,
}) {
  useEffect(() => {
    const onKeyDown = event => {
      const mod = event.metaKey || event.ctrlKey;
      const key = event.key.toLowerCase();
      // ⌘⇧S 折叠 / 展开导航栏(⌘B 归资源栏,由系统菜单占着)
      if (mod && event.shiftKey && key === "s") {
        event.preventDefault();
        onToggleNav?.();
        return;
      }
      if (mod && !event.shiftKey && key === "k" && paneAvailable) {
        event.preventDefault();
        onOpenSearch();
        return;
      }
      // ⌘F 聚焦资源栏那条常驻搜索框;跨库全文仍走 ⌘K
      if (mod && !event.shiftKey && key === "f" && paneAvailable) {
        event.preventDefault();
        onFocusPaneSearch?.();
        return;
      }
      if (event.key === "Escape") {
        const active = dismissers.find(item => item.open);
        active?.dismiss();
        return;
      }
      if (document.activeElement
          && ["INPUT", "TEXTAREA"].includes(document.activeElement.tagName)) {
        return;
      }

      const overlayOpen = dismissers.some(item => item.open);
      if (!overlayOpen && view === "library" && currentSession) {
        if (event.key === "F2") {
          event.preventDefault();
          onRename(currentSession);
          return;
        }
        if (event.key === "Backspace" || event.key === "Delete") {
          event.preventDefault();
          if (multiIds.length > 1) {
            onBatchDelete(multiIds.map(key => sessionsByKey[key]).filter(Boolean));
          } else {
            onDelete(currentSession);
          }
          return;
        }
        if (event.key === "Enter") {
          event.preventDefault();
          onResume(currentSession);
          return;
        }
      }

      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      event.preventDefault();
      const ids = view === "library"
        ? libraryVisibleIds
        : view === "history"
          ? historyVisibleIds
          : [];
      if (!ids.length) return;
      const selected = view === "library" ? selectedSessionId : selectedHistoryId;
      const currentIndex = ids.indexOf(selected);
      const step = event.key === "ArrowDown" ? 1 : -1;
      const nextIndex = currentIndex < 0
        ? 0
        : Math.max(0, Math.min(ids.length - 1, currentIndex + step));
      if (view === "library") selectSession(ids[nextIndex]);
      else selectHistory(ids[nextIndex]);
    };

    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [
    currentSession,
    dismissers,
    historyVisibleIds,
    libraryVisibleIds,
    multiIds,
    onBatchDelete,
    onDelete,
    onFocusPaneSearch,
    onOpenSearch,
    onRename,
    onToggleNav,
    onResume,
    paneAvailable,
    selectHistory,
    selectSession,
    selectedHistoryId,
    selectedSessionId,
    sessionsByKey,
    view,
  ]);
}
