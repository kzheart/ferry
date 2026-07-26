import { useEffect, useRef, useState } from "react";
import { engine } from "../../platform/desktop/client.js";
import { cacheGet, cacheSet } from "../../platform/desktop/cache.js";

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
  const [scanReady, setScanReady] = useState(false);
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
      const result = await engine("scan");
      setScan(result);
      setScanReady(true);
      persist({ scan: result });
    }
    catch (error) {
      setScanReady(false);
      setScan(current => ({
        tools: {},
        sessions: [],
        error: error.message,
        ...(current || {}),
      }));
    }
    setScanning(false);
  };
  const loadHistory = () => engine("history")
    .then(rows => { setHistoryRows(rows); persist({ history: rows }); })
    .catch(() => {});
  // 只删 Ferry 自己的迁移记录,已迁到目标工具里的会话不受影响
  const deleteHistory = id => engine("history_delete", { id }).then(loadHistory);
  const loadPricing = () => engine("pricing")
    .then(p => { setPricing(p); persist({ pricing: p }); })
    .catch(() => {});

  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    engine("env").then(e => { setEnv(e); persist({ env: e }); }).catch(() => {});
    doScan();
    loadHistory();
    loadPricing();
  }, []);

  return { env, scan, scanning, scanReady, historyRows,
    pricing, doScan, loadHistory, deleteHistory };
}

/** 扫描进度单独订阅:进度每 350ms 变一次,挂在根组件上等于全树重渲染。
 *  由真正显示进度条的组件自己调用。 */
export function useScanProgress(scanning) {
  const [progress, setProgress] = useState(null);
  useEffect(() => {
    if (!scanning) { setProgress(null); return undefined; }
    // scan 阻塞在 serial 池,scan_progress 走 parallel-read 池,扫描期间可查
    const poll = setInterval(() => {
      engine("scan_progress")
        .then(next => { if (next?.state === "running") setProgress(next); })
        .catch(() => {});
    }, 350);
    return () => { clearInterval(poll); setProgress(null); };
  }, [scanning]);
  return progress;
}
