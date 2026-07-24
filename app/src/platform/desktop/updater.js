export const isNativeApp = () =>
  typeof window !== "undefined" && !!window.__TAURI_INTERNALS__;

export async function getAppVersion() {
  const { getVersion } = await import("@tauri-apps/api/app");
  return getVersion();
}

export async function checkAppUpdate(options) {
  const { check } = await import("@tauri-apps/plugin-updater");
  return check(options);
}

export async function downloadAppUpdate(update, onEvent) {
  return update.download(onEvent);
}

export async function installAppUpdate(update) {
  return update.install();
}

export async function closeAppUpdate(update) {
  return update?.close();
}

export async function relaunchApp() {
  const { relaunch } = await import("@tauri-apps/plugin-process");
  return relaunch();
}
