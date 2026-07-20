import { invoke } from "@tauri-apps/api/core";

const inTauri = () => !!window.__TAURI_INTERNALS__;

export async function rpc(method, params) {
  const request = JSON.stringify({ method, params: params || {} });
  const raw = inTauri()
    ? await invoke("engine_rpc", { request })
    : await (await fetch("/api/rpc", { method: "POST", body: request })).text();
  const response = JSON.parse(raw);
  if (!response.ok) throw new Error(response.error || "引擎调用失败");
  return response.result;
}

// The desktop command currently accepts the existing launch DTO as-is.
export const openTerminal = launch =>
  inTauri() ? invoke("open_terminal", { launch }) : Promise.resolve();

export const revealPath = path =>
  inTauri() ? invoke("reveal_path", { path }) : Promise.resolve();

export const canReveal = () => inTauri();
