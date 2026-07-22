import { useEffect, useRef, useState } from "react";
import { rpc } from "../../api/transport/rpc.js";
import { cacheGet, cacheSet } from "../../api/platform/idbCache.js";

// 秒开:上次结果落 IndexedDB;main.jsx 挂载前先 preloadBrowserCache 预读(毫秒级),
// 首帧即带旧数据渲染,引擎就绪后后台静默刷新
const CACHE_KEY = "browser-data";

let preloaded = null;
export function preloadBrowserCache() {
  return cacheGet(CACHE_KEY).then(cached => { preloaded = cached || null; });
}

export function useBrowserData() {
  const [env, setEnv] = useState(() => preloaded?.env || null);
  const [scan, setScan] = useState(() => preloaded?.scan || null);
  const [scanning, setScanning] = useState(false);
  const [lastScan, setLastScan] = useState(() => preloaded?.lastScan || null);
  const [historyRows, setHistoryRows] = useState(() => preloaded?.history || []);
  const [pricing, setPricing] = useState(() => preloaded?.pricing || null);
  const booted = useRef(false);
  // 以缓存为底,引擎新鲜结果逐字段覆盖后整体回写,启动中途退出也不丢字段
  const cache = useRef(null);
  if (cache.current === null) cache.current = { ...(preloaded || {}) };

  const persist = patch => {
    Object.assign(cache.current, patch);
    cacheSet(CACHE_KEY, { ...cache.current });
  };

  const doScan = async () => {
    if (scanning) return;
    setScanning(true);
    try {
      const result = await rpc("scan");
      const now = Date.now();
      setScan(result); setLastScan(now);
      persist({ scan: result, lastScan: now });
    }
    catch (error) { setScan(current => ({ tools: {}, sessions: [], error: error.message, ...(current || {}) })); }
    setScanning(false);
  };
  const loadHistory = () => rpc("history")
    .then(rows => { setHistoryRows(rows); persist({ history: rows }); })
    .catch(() => {});
  const loadPricing = () => rpc("pricing")
    .then(p => { setPricing(p); persist({ pricing: p }); })
    .catch(() => {});

  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    rpc("env").then(e => { setEnv(e); persist({ env: e }); }).catch(() => {});
    doScan();
    loadHistory();
    loadPricing();
    // 旧版曾把缓存写进 localStorage(超配额写不进),清掉残留
    try { localStorage.removeItem("ferry-data-cache"); } catch { /* 忽略 */ }
  }, []);

  return { env, scan, scanning, lastScan, historyRows, pricing,
    doScan, loadHistory };
}
