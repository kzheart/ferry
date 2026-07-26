// ⌘K 的全文检索:前端过滤只看得见标题,这里向 Engine 要正文命中。
// 输入抖动大,300ms 防抖;响应里回显的 query 与当前输入不一致就丢弃。
import { useEffect, useRef, useState } from "react";

import { engine } from "../platform/desktop/client.js";

const DEBOUNCE_MS = 300;
const MIN_QUERY_CHARS = 2;
const RESULT_LIMIT = 20;

export function useSessionContentSearch(query, enabled) {
  const [result, setResult] = useState(null);
  const latestQuery = useRef("");

  useEffect(() => {
    const trimmed = (query || "").trim();
    latestQuery.current = trimmed;
    if (!enabled || trimmed.length < MIN_QUERY_CHARS) {
      setResult(null);
      return;
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
          setResult(data);
        })
        // 索引没就绪或检索失败都不该让搜索框空掉,静默退回纯前端过滤
        .catch(() => { if (!cancelled) setResult(null); });
    }, DEBOUNCE_MS);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [query, enabled]);

  return result;
}
