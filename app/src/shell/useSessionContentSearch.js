// ⌘K 的全文检索:前端过滤只看得见标题,这里向 Engine 要正文命中。
// 输入抖动大,300ms 防抖;响应里回显的 query 与当前输入不一致就丢弃。
import { useEffect, useRef, useState } from "react";

import { engine } from "../platform/desktop/client.js";

const DEBOUNCE_MS = 300;
const MIN_QUERY_CHARS = 2;
const RESULT_LIMIT = 20;

export function useSessionContentSearch(query, enabled) {
  const [payload, setPayload] = useState({ query: "", status: "idle", data: null });
  const latestQuery = useRef("");
  const trimmed = (query || "").trim();
  const active = Boolean(enabled && trimmed.length >= MIN_QUERY_CHARS);

  useEffect(() => {
    latestQuery.current = trimmed;
    if (!active) {
      setPayload({ query: trimmed, status: "idle", data: null });
      return undefined;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      engine("session_search", {
        query: trimmed,
        scope: "content",
        limit: RESULT_LIMIT,
      })
        .then(data => {
          // Engine 回显 query;对不上说明这条响应已被后续按键取代
          if (cancelled || data?.query !== latestQuery.current) return;
          setPayload({ query: trimmed, status: "ready", data });
        })
        // 索引没就绪或检索失败都不该让搜索框空掉,静默退回纯前端过滤
        .catch(() => {
          if (!cancelled && latestQuery.current === trimmed) {
            setPayload({ query: trimmed, status: "ready", data: null });
          }
        });
    }, DEBOUNCE_MS);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [active, trimmed]);

  const matches = payload.query === trimmed;
  // 查询词一变,还没等到这篇的响应之前都算在搜:不能先画「无匹配」。
  const pending = active && (!matches || payload.status !== "ready");
  const result = matches && payload.status === "ready" ? payload.data : null;
  return { pending, result };
}
