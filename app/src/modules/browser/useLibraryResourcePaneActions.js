import { useCallback, useRef } from "react";

/**
 * 会话库资源栏的行级交互。
 *
 * 这里不拥有会话详情、元数据写入或删除事务；它只把列表手势转换为上层协调器
 * 提供的领域动作，确保虚拟化行可以获得稳定的回调身份。
 */
export function useLibraryResourcePaneActions({
  sessionsByKey,
  selectedId,
  visibleIds,
  multiIds,
  setMultiIds,
  onSelect,
  onTogglePin,
  onDelete,
  onOpenMenu,
}) {
  const actions = useRef({});
  actions.current.click = (key, event) => {
    if (event.metaKey || event.ctrlKey) {
      setMultiIds(selected => {
        const base = selected.length ? selected : (selectedId ? [selectedId] : []);
        return base.includes(key)
          ? base.filter(value => value !== key) : [...base, key];
      });
      return;
    }
    if (event.shiftKey && selectedId) {
      const start = visibleIds.indexOf(selectedId);
      const end = visibleIds.indexOf(key);
      if (start >= 0 && end >= 0) {
        setMultiIds(visibleIds.slice(Math.min(start, end), Math.max(start, end) + 1));
        return;
      }
    }
    setMultiIds([]);
    onSelect(key);
  };
  actions.current.more = (key, event) => {
    const position = event.type === "contextmenu"
      ? { x: event.clientX, y: event.clientY }
      : (() => {
          const rect = event.currentTarget.getBoundingClientRect();
          return { x: rect.right - 208, y: rect.bottom + 4 };
        })();
    if (multiIds.length > 1 && multiIds.includes(key)) {
      onOpenMenu({ ...position, key, multi: true });
      return;
    }
    setMultiIds([]);
    if (key !== selectedId) onSelect(key);
    onOpenMenu({ ...position, key });
  };
  actions.current.pin = key => {
    const session = sessionsByKey[key];
    if (session) onTogglePin(session);
  };
  actions.current.delete = key => {
    const session = sessionsByKey[key];
    if (session) onDelete(session);
  };

  return {
    onRowClick: useCallback((key, event) => actions.current.click(key, event), []),
    onRowMore: useCallback((key, event) => actions.current.more(key, event), []),
    onRowPin: useCallback(key => actions.current.pin(key), []),
    onRowDelete: useCallback(key => actions.current.delete(key), []),
  };
}
