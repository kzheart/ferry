// 启动缓存的 KV 存取(IndexedDB):scan 结果按 UTF-16 计超过 localStorage 5MB 配额,
// 这里走 IndexedDB 结构化克隆,免序列化且配额充裕。读写失败一律静默,缓存只是加速。
const DB = "ferry-cache";
const STORE = "kv";

function withStore(mode, fn) {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open(DB, 1);
    open.onupgradeneeded = () => open.result.createObjectStore(STORE);
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const db = open.result;
      const tx = db.transaction(STORE, mode);
      const req = fn(tx.objectStore(STORE));
      tx.oncomplete = () => { db.close(); resolve(req?.result); };
      tx.onerror = () => { db.close(); reject(tx.error); };
    };
  });
}

export const cacheGet = key =>
  withStore("readonly", store => store.get(key)).catch(() => undefined);

export const cacheSet = (key, value) =>
  withStore("readwrite", store => store.put(value, key)).catch(() => {});
