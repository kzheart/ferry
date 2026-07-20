import { useCallback, useEffect, useRef, useState } from "react";
import { checkAppUpdate, closeAppUpdate, downloadAppUpdate, getAppVersion,
  installAppUpdate, isNativeApp, relaunchApp } from "../../api/platform/appUpdater.js";

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

export function formatBytes(bytes) {
  if (!Number.isFinite(bytes)) return "—";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes;
  let unit = -1;
  do { value /= 1024; unit += 1; } while (value >= 1024 && unit < units.length - 1);
  return `${value.toFixed(value >= 10 ? 1 : 2)} ${units[unit]}`;
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

  const downloadUpdate = useCallback(async () => {
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
        } else if (event.event === "Finished") {
          setState(v => ({ ...v, phase: "downloaded", downloaded: v.total ?? downloaded }));
        }
      });
    } catch (error) {
      setState(v => ({ ...v, phase: "error", error: messageOf(error), failedAction: "download" }));
    } finally { busyRef.current = false; }
  }, []);

  const installAndRestart = useCallback(async () => {
    const update = updateRef.current;
    const retryingInstall = state.phase === "error" && state.failedAction === "install";
    if (!update || (state.phase !== "downloaded" && !retryingInstall) || busyRef.current) return;
    busyRef.current = true;
    setState(v => ({ ...v, phase: "installing", error: null, failedAction: null }));
    try {
      await installAppUpdate(update);
      await relaunchApp();
    } catch (error) {
      busyRef.current = false;
      setState(v => ({ ...v, phase: "error", error: messageOf(error), failedAction: "install" }));
    }
  }, [state.failedAction, state.phase]);

  useEffect(() => {
    if (!native || !autoCheck) return;
    const timer = window.setTimeout(checkForUpdate, delay);
    return () => window.clearTimeout(timer);
  }, [autoCheck, checkForUpdate, delay, native]);

  useEffect(() => () => { closeAppUpdate(updateRef.current).catch(() => {}); }, []);

  return { ...state, checkForUpdate, downloadUpdate, installAndRestart };
}
