import { useCallback, useEffect, useRef, useState } from "react";
import { checkAppUpdate, closeAppUpdate, downloadAppUpdate, getAppVersion,
  installAppUpdate, isNativeApp, relaunchApp } from "../../platform/desktop/updater.js";

// 更新装完就重启,新进程里没有任何内存状态能告诉我们「刚更新过」。
// 所以安装前把版本与说明落到 localStorage,启动时读到就弹公告,读完即删。
const PENDING_KEY = "ferry-update-pending";

const INITIAL = {
  phase: "idle",
  currentVersion: "—",
  update: null,
  downloaded: 0,
  total: null,
  error: null,
  failedAction: null,
  supported: false,
};

const messageOf = error => error instanceof Error ? error.message : String(error);

function readPending() {
  try {
    const raw = localStorage.getItem(PENDING_KEY);
    if (!raw) return null;
    const value = JSON.parse(raw);
    return value && typeof value.to === "string" ? value : null;
  } catch {
    return null;
  }
}

// 更新完成公告:上一轮安装留下的记录,展示一次就清掉
export function useUpdateAnnouncement() {
  const [announcement, setAnnouncement] = useState(() => readPending());
  const dismiss = useCallback(() => {
    try { localStorage.removeItem(PENDING_KEY); } catch { /* 存不进去就只影响这次 */ }
    setAnnouncement(null);
  }, []);
  return { announcement, dismiss };
}

export function useAppUpdater(autoCheck, delay = 3500) {
  const [state, setState] = useState(INITIAL);
  const updateRef = useRef(null);
  const busyRef = useRef(false);
  const native = isNativeApp();

  useEffect(() => {
    if (!native) return;
    getAppVersion()
      .then(currentVersion => setState(v => ({ ...v, currentVersion, supported: true })))
      .catch(error => setState(v => ({ ...v, error: messageOf(error) })));
  }, [native]);

  const checkForUpdate = useCallback(async () => {
    if (!native || busyRef.current) return;
    busyRef.current = true;
    setState(v => ({ ...v, phase: "checking", error: null, update: null,
      downloaded: 0, total: null, failedAction: null }));
    try {
      if (updateRef.current) await closeAppUpdate(updateRef.current).catch(() => {});
      const update = await checkAppUpdate({ timeout: 15000 });
      updateRef.current = update;
      setState(v => ({ ...v, phase: update ? "available" : "upToDate", update: update ? {
        version: update.version, date: update.date, body: update.body || ""
      } : null }));
    } catch (error) {
      setState(v => ({ ...v, phase: "error", error: messageOf(error), failedAction: "check" }));
    } finally { busyRef.current = false; }
  }, [native]);

  // 一次点击走完下载 + 安装 + 重启。中途不再问第二次:用户点下载图标时
  // 表达的就是「装上」,多一次确认只是把同一个决定拆成两半。
  const startUpdate = useCallback(async () => {
    const update = updateRef.current;
    if (!update || busyRef.current) return;
    busyRef.current = true;
    setState(v => ({ ...v, phase: "downloading", downloaded: 0, total: null, error: null,
      failedAction: null }));
    let downloaded = 0;
    try {
      await downloadAppUpdate(update, event => {
        if (event.event === "Started") {
          setState(v => ({ ...v, total: event.data.contentLength ?? null }));
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          setState(v => ({ ...v, downloaded }));
        }
      });
    } catch (error) {
      busyRef.current = false;
      setState(v => ({ ...v, phase: "error", error: messageOf(error), failedAction: "update" }));
      return;
    }
    setState(v => {
      // 重启后要展示的公告:趁进程还活着写进去
      try {
        localStorage.setItem(PENDING_KEY, JSON.stringify({
          from: v.currentVersion,
          to: update.version,
          date: update.date || null,
          notes: update.body || "",
        }));
      } catch { /* 写不进去就只是少一次公告,不该拦住安装 */ }
      return { ...v, phase: "installing", downloaded: v.total ?? downloaded };
    });
    try {
      await installAppUpdate(update);
      await relaunchApp();
    } catch (error) {
      try { localStorage.removeItem(PENDING_KEY); } catch { /* 同上 */ }
      busyRef.current = false;
      setState(v => ({ ...v, phase: "error", error: messageOf(error), failedAction: "update" }));
    }
  }, []);

  useEffect(() => {
    if (!native || !autoCheck) return;
    const timer = window.setTimeout(checkForUpdate, delay);
    return () => window.clearTimeout(timer);
  }, [autoCheck, checkForUpdate, delay, native]);

  useEffect(() => () => { closeAppUpdate(updateRef.current).catch(() => {}); }, []);

  const progress = state.total ? Math.min(1, state.downloaded / state.total) : null;

  return { ...state, progress, checkForUpdate, startUpdate };
}
