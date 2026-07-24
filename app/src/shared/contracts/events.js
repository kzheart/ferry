import i18n from "../i18n/index.js";

const t = (key, params) => i18n.t(key, params);

export function renderEvent(event) {
  if (!event || typeof event !== "object" || Array.isArray(event)) return "";
  if (typeof event.code !== "string" || !event.code) return "";
  const params = event.params && typeof event.params === "object"
    ? event.params
    : {};
  return t(`events:${event.code}`, { ...params, defaultValue: event.code });
}

export const renderEvents = list => (list || []).map(renderEvent);

const probeTextFor = (code, params) => {
  const p = params || {};
  if (code === "probe.process_failed") {
    if (p.exit_code != null) {
      return t("events:probe.process_failed_with_code", { exit_code: p.exit_code });
    }
    return t("events:probe.process_failed", { exit_code: "" });
  }
  const key = `events:probe.${code}`;
  const v = t(key, { ...p, defaultValue: null });
  return v != null ? v : code;
};

export const probeFailed = p =>
  !!p && (p.status === "failed" || p.ok === false);

export function probeText(p) {
  if (!p) return "";
  if (p.detail != null) return p.detail;
  const parts = [];
  if (p.isolation) {
    const kind = t(`events:isolation.${p.isolation.kind}`, { defaultValue: p.isolation.kind });
    parts.push(t("events:probe.isolation_cleanup", { kind, id: p.isolation.id ?? "" }));
  }
  if (p.code) parts.push(probeTextFor(p.code, p.params || {}));
  const d = p.diagnostic || {};
  if (d.stdout) parts.push(d.stdout);
  if (d.stderr) parts.push(d.stderr);
  if (d.truncated) parts.push(t("events:probe.truncated_suffix"));
  return parts.filter(Boolean).join("\n");
}
