import { useEffect, useMemo, useRef, useState } from "react";
import { engine } from "../../platform/desktop/client.js";
import { isOpaqueSessionRef } from "../../shared/contracts/generated/session-ref.js";
import { repoOf, sessionRef } from "./sessionModel.js";
import { sessionIdentity } from "./sessionAttachment.js";

const DETAIL_CACHE_LIMIT = 30;

export function useSessionSelection({
  sessions,
  ready,
  onSelect,
  onFallbackLoad,
  onStaleReference,
}) {
  const [selectedId, setSelectedId] = useState(null);
  const [detail, setDetail] = useState(null);
  const [refreshing, setRefreshing] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const detailCache = useRef(new Map());

  const sessionsByKey = useMemo(
    () =>
      Object.fromEntries(
        sessions.map((session) => [sessionIdentity(session), session]),
      ),
    [sessions],
  );

  const cacheDetail = (id, data) => {
    const cache = detailCache.current;
    cache.delete(id);
    cache.set(id, data);
    if (cache.size > DETAIL_CACHE_LIMIT)
      cache.delete(cache.keys().next().value);
  };

  const loadDetail = (key, session, cachedData = null) => {
    const ref = sessionRef(session);
    setDetail({
      id: key,
      ref,
      data: cachedData,
    });
    engine("show", { tool: session.tool, ref, from_message: 1, limit: 30 })
      .then((data) => {
        cacheDetail(key, data);
        setDetail((current) =>
          current?.id === key && current.ref === ref
            ? { id: key, ref, data }
            : current,
        );
      })
      .catch((error) => {
        setDetail((current) =>
          current?.id === key && current.ref === ref
            ? { ...current, error: error.message }
            : current,
        );
        if (error?.code === "agent.reference_invalid") {
          onStaleReference();
        }
      });
  };

  const select = (key) => {
    setSelectedId(key);
    onSelect();
    const session =
      sessionsByKey[key] ||
      sessions.find((item) => sessionIdentity(item) === key);
    if (!session) return;
    const cachedData = detailCache.current.get(key) || null;
    if (ready) {
      loadDetail(key, session, cachedData);
    } else {
      setDetail({ id: key, ref: null, data: cachedData });
    }
  };

  useEffect(() => {
    if (!ready || !selectedId) return;
    const session = sessionsByKey[selectedId];
    if (!session) return;
    const ref = sessionRef(session);
    if (detail?.id === selectedId && detail.ref === ref) return;
    loadDetail(
      selectedId,
      session,
      detailCache.current.get(selectedId) || null,
    );
  }, [ready, selectedId, sessionsByKey, detail?.id, detail?.ref]);

  const loadEntitySession = (action, entity) => {
    const candidate = sessions.find(
      (session) =>
        (action.sessionId &&
          session.tool === action.tool &&
          session.id === action.sessionId) ||
        (action.ref && sessionRef(session) === action.ref) ||
        (entity?.title &&
          session.tool === action.tool &&
          session.title === entity.title &&
          (!entity.project || repoOf(session.dir) === entity.project)),
    );
    if (candidate) {
      const key = sessionIdentity(candidate);
      select(key);
      return key;
    }
    if (action.tool && isOpaqueSessionRef(action.ref)) {
      const key = `${action.tool}\u0000${action.ref}`;
      setSelectedId(key);
      onSelect();
      setDetail({ id: key, ref: action.ref, data: null });
      engine("show", {
        tool: action.tool,
        ref: action.ref,
        from_message: 1,
        limit: 30,
      })
        .then((data) =>
          setDetail((current) =>
            current?.id === key ? { id: key, ref: action.ref, data } : current,
          ),
        )
        .catch((error) =>
          setDetail((current) =>
            current?.id === key
              ? { ...current, error: error.message }
              : current,
          ),
        );
      onFallbackLoad();
      return key;
    }
    return null;
  };

  const refreshDetail = async () => {
    const session =
      selectedId &&
      (sessionsByKey[selectedId] ||
        sessions.find((item) => sessionIdentity(item) === selectedId));
    if (!session || refreshing) return;
    setRefreshing(true);
    try {
      const data = await engine("show", {
        tool: session.tool,
        ref: sessionRef(session),
        from_message: 1,
        limit: 30,
      });
      cacheDetail(selectedId, data);
      setDetail((current) =>
        current?.id === selectedId
          ? { id: selectedId, ref: sessionRef(session), data }
          : current,
      );
    } catch (error) {
      setDetail((current) =>
        current?.id === selectedId
          ? { ...current, error: error.message }
          : current,
      );
      if (error?.code === "agent.reference_invalid") {
        onStaleReference();
      }
    }
    setRefreshing(false);
  };

  const loadMore = async () => {
    const current = detail;
    if (loadingMore || !current?.data?.next_from_message) return;
    const session =
      sessionsByKey[current.id] ||
      sessions.find((item) => sessionIdentity(item) === current.id);
    if (!session) return;
    setLoadingMore(true);
    try {
      const page = await engine("show", {
        tool: session.tool,
        ref: sessionRef(session),
        from_message: current.data.next_from_message,
        limit: 30,
      });
      setDetail((active) =>
        active?.id === current.id && active.ref === current.ref
          ? {
              ...active,
              data: {
                ...page,
                messages: [
                  ...(active.data?.messages || []),
                  ...(page.messages || []),
                ],
                turns: [...(active.data?.turns || []), ...(page.turns || [])],
                returned_message_count:
                  (active.data?.returned_message_count || 0) +
                  (page.returned_message_count || 0),
                context_compactions: active.data?.context_compactions || [],
              },
            }
          : active,
      );
    } catch (error) {
      setDetail((active) =>
        active?.id === current.id && active.ref === current.ref
          ? { ...active, error: error.message }
          : active,
      );
    } finally {
      setLoadingMore(false);
    }
  };

  const clearSelection = () => {
    setSelectedId(null);
    setDetail(null);
  };

  const discardCachedDetail = (session) => {
    detailCache.current.delete(sessionIdentity(session));
  };

  return {
    selectedId,
    setSelectedId,
    detail,
    refreshing,
    loadingMore,
    sessionsByKey,
    select,
    loadEntitySession,
    refreshDetail,
    loadMore,
    clearSelection,
    discardCachedDetail,
  };
}
