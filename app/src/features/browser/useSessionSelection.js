import { useMemo, useRef, useState } from "react";
import { rpc } from "../../api/transport/rpc.js";
import { repoOf, sessionRef } from "../../domain/sessions/sessionModel.js";
import { sessionIdentity } from "../../domain/sessions/sessionAttachment.js";

const DETAIL_CACHE_LIMIT = 30;

export function useSessionSelection({ sessions, onSelect, onFallbackLoad }) {
  const [selectedId, setSelectedId] = useState(null);
  const [detail, setDetail] = useState(null);
  const [refreshing, setRefreshing] = useState(false);
  const detailCache = useRef(new Map());

  const sessionsByKey = useMemo(
    () => Object.fromEntries(sessions.map(session => [sessionIdentity(session), session])),
    [sessions],
  );

  const cacheDetail = (id, data) => {
    const cache = detailCache.current;
    cache.delete(id);
    cache.set(id, data);
    if (cache.size > DETAIL_CACHE_LIMIT) cache.delete(cache.keys().next().value);
  };

  const select = key => {
    setSelectedId(key);
    onSelect();
    const session = sessionsByKey[key] || sessions.find(item => sessionIdentity(item) === key);
    if (!session) return;
    setDetail({ id: key, data: detailCache.current.get(key) || null });
    rpc("show", { tool: session.tool, ref: sessionRef(session) })
      .then(data => {
        cacheDetail(key, data);
        setDetail(current => current?.id === key ? { ...current, data } : current);
      })
      .catch(error => setDetail(current => current?.id === key
        ? { ...current, error: error.message }
        : current));
  };

  const loadEntitySession = (action, entity) => {
    const candidate = sessions.find(session =>
      (action.sessionId && session.tool === action.tool && session.id === action.sessionId) ||
      (action.ref && sessionRef(session) === action.ref) ||
      (entity?.title && session.tool === action.tool &&
        session.title === entity.title &&
        (!entity.project || repoOf(session.dir) === entity.project)));
    if (candidate) {
      const key = sessionIdentity(candidate);
      select(key);
      return key;
    }
    if (action.tool && (action.ref || action.sessionId)) {
      const key = sessionIdentity({ tool: action.tool, id: action.sessionId || action.ref });
      setSelectedId(key);
      onSelect();
      setDetail({ id: key, data: null });
      rpc("show", { tool: action.tool, ref: action.ref || action.sessionId })
        .then(data => setDetail(current => current?.id === key ? { ...current, data } : current))
        .catch(error => setDetail(current => current?.id === key
          ? { ...current, error: error.message }
          : current));
      onFallbackLoad();
      return key;
    }
    return null;
  };

  const refreshDetail = async () => {
    const session = selectedId && (sessionsByKey[selectedId]
      || sessions.find(item => sessionIdentity(item) === selectedId));
    if (!session || refreshing) return;
    setRefreshing(true);
    try {
      const data = await rpc("show", { tool: session.tool, ref: sessionRef(session) });
      cacheDetail(selectedId, data);
      setDetail(current => current?.id === selectedId ? { id: selectedId, data } : current);
    } catch (error) {
      setDetail(current => current?.id === selectedId
        ? { ...current, error: error.message }
        : current);
    }
    setRefreshing(false);
  };

  const clearSelection = () => {
    setSelectedId(null);
    setDetail(null);
  };

  const discardCachedDetail = session => {
    detailCache.current.delete(sessionIdentity(session));
  };

  return {
    selectedId,
    setSelectedId,
    detail,
    refreshing,
    sessionsByKey,
    select,
    loadEntitySession,
    refreshDetail,
    clearSelection,
    discardCachedDetail,
  };
}
