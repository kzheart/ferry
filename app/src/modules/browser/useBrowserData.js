import { useEffect, useRef, useState } from "react";
import { engine, onEngineEvent } from "../../platform/desktop/client.js";
import { cacheGet, cacheSet } from "../../platform/desktop/cache.js";
import { sessionIdentity } from "./sessionAttachment.js";

// 增量合并以 ref 为主键;若引擎侧同一会话曾被换发过 ref(不该发生,但一旦
// 发生 UI 会出现两行同 key 的鬼影行),按会话身份收敛到最新的一行兜底。
export function mergeSessionsDelta(sessions, payload) {
  const byRef = new Map(sessions.map(s => [s.ref, s]));
  for (const row of payload.upserts || []) byRef.set(row.ref, row);
  for (const ref of payload.removals || []) byRef.delete(ref);
  const byId = new Map();
  for (const row of byRef.values()) {
    const key = sessionIdentity(row) || row.ref;
    const prev = byId.get(key);
    if (!prev || !(row.updated < prev.updated)) byId.set(key, row);
  }
  return [...byId.values()].sort((a, b) =>
    a.updated < b.updated ? 1 : a.updated > b.updated ? -1 : 0);
}

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

  // 增量推送的合并基线:只信本进程引擎返回的 generation,
  // IndexedDB 里的旧值可能来自上一个引擎进程,代际号会撞车。
  const scanGen = useRef(null);
  // scan 状态的同步镜像:delta 到达时要在推进 generation 之前判断
  // 列表是否就绪,不能依赖 setState updater 的执行时机。
  const scanMirror = useRef(scan);
  useEffect(() => { scanMirror.current = scan; }, [scan]);

  const doScan = async () => {
    if (scanning) return;
    setScanning(true);
    try {
      const result = await engine("scan");
      scanGen.current = result.generation ?? null;
      scanMirror.current = result;
      setScan(result);
      setScanReady(true);
      persist({ scan: result });
    }
    catch (error) {
      setScanReady(false);
      // 保留上次扫到的会话继续可用,但 error 必须是这次的——
      // 展开写在最前面,否则上一次失败留下的旧 error 会盖掉新的
      setScan(current => ({
        tools: {},
        sessions: [],
        ...(current || {}),
        error: error.message || String(error),
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

  // 引擎活索引推送:sessions.changed 增量直接并入列表,列表常新,
  // 不再依赖用户点刷新。代际断档(错过事件/引擎重启)时静默全量重拉。
  useEffect(() => {
    // 增量每次全量落盘太重;活跃会话期间事件密集,限频回写足够安全
    // (掉电最多丢十秒的列表缓存,下次启动引擎会补齐)。
    let persistTimer = null;
    const persistSoon = () => {
      if (persistTimer !== null) return;
      persistTimer = setTimeout(() => {
        persistTimer = null;
        persist({ scan: cachePending.value });
      }, 10_000);
    };
    const cachePending = { value: null };

    const silentRescan = () => engine("scan").then(result => {
      scanGen.current = result.generation ?? null;
      scanMirror.current = result;
      setScan(result);
      setScanReady(true);
      persist({ scan: result });
    }).catch(() => {});

    const applyDelta = payload => {
      const expected = scanGen.current;
      // 列表未就绪时若只推进 generation,这条 delta 等于被丢弃,
      // 后续 delta 会顺着断掉的链继续通过校验——一并走全量重拉。
      if (
        expected == null
        || payload.generation !== expected + 1
        || !scanMirror.current?.sessions
      ) {
        silentRescan();
        return;
      }
      scanGen.current = payload.generation;
      const sessions = mergeSessionsDelta(
        scanMirror.current.sessions, payload,
      );
      const next = {
        ...scanMirror.current, sessions, generation: payload.generation,
      };
      scanMirror.current = next;
      setScan(next);
      cachePending.value = next;
      persistSoon();
    };

    let disposed = false;
    let unlisten = null;
    onEngineEvent(event => {
      if (event.type === "sessions.changed") applyDelta(event.payload || {});
    }).then(dispose => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
      if (persistTimer !== null) clearTimeout(persistTimer);
    };
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
