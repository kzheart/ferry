import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import zhCNCommon from "../locales/zh-CN/common.json";
import enCommon from "../locales/en/common.json";
import zhCNErrors from "../locales/zh-CN/errors.json";
import enErrors from "../locales/en/errors.json";
import zhCNEvents from "../locales/zh-CN/events.json";
import enEvents from "../locales/en/events.json";
import zhCNBrowser from "../locales/zh-CN/browser.json";
import enBrowser from "../locales/en/browser.json";
import zhCNMigration from "../locales/zh-CN/migration.json";
import enMigration from "../locales/en/migration.json";
import zhCNSnapshots from "../locales/zh-CN/snapshots.json";
import enSnapshots from "../locales/en/snapshots.json";
import zhCNOnboarding from "../locales/zh-CN/onboarding.json";
import enOnboarding from "../locales/en/onboarding.json";
import zhCNSettings from "../locales/zh-CN/settings.json";
import enSettings from "../locales/en/settings.json";
import zhCNOverlays from "../locales/zh-CN/overlays.json";
import enOverlays from "../locales/en/overlays.json";
import zhCNApp from "../locales/zh-CN/app.json";
import enApp from "../locales/en/app.json";
import zhCNOverview from "../locales/zh-CN/overview.json";
import enOverview from "../locales/en/overview.json";

export const SUPPORTED_LOCALES = ["zh-CN", "en"];
export const FALLBACK_LOCALE = "zh-CN";
export const DEFAULT_LOCALE = null;

const RESOURCES = {
  "zh-CN": { common: zhCNCommon, errors: zhCNErrors, events: zhCNEvents, browser: zhCNBrowser, migration: zhCNMigration, snapshots: zhCNSnapshots, onboarding: zhCNOnboarding, settings: zhCNSettings, overlays: zhCNOverlays, app: zhCNApp, overview: zhCNOverview },
  "en": { common: enCommon, errors: enErrors, events: enEvents, browser: enBrowser, migration: enMigration, snapshots: enSnapshots, onboarding: enOnboarding, settings: enSettings, overlays: enOverlays, app: enApp, overview: enOverview },
};

const SETTINGS_KEY = "ferry-settings";

function readStoredLocale() {
  try {
    const s = JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}");
    return s.locale ?? null;
  } catch { return null; }
}

export function matchSystemLocale() {
  const nav = (navigator.language || "").toLowerCase();
  if (nav.startsWith("zh")) return "zh-CN";
  if (nav.startsWith("en")) return "en";
  return FALLBACK_LOCALE;
}

export function resolveLocale() {
  const stored = readStoredLocale();
  return stored || matchSystemLocale();
}

export function normalizeLocale(locale) {
  if (!locale) return matchSystemLocale();
  const lc = String(locale).toLowerCase();
  if (lc.startsWith("zh")) return "zh-CN";
  if (lc.startsWith("en")) return "en";
  return FALLBACK_LOCALE;
}

let initialized = false;

export function initI18n() {
  if (initialized) return i18n;
  initialized = true;
  const lng = resolveLocale();
  i18n
    .use(initReactI18next)
    .init({
      resources: RESOURCES,
      lng,
      fallbackLng: FALLBACK_LOCALE,
      defaultNS: "common",
      ns: ["common", "errors", "events", "browser", "migration", "snapshots", "onboarding", "settings", "overlays", "app", "overview"],
      interpolation: { escapeValue: false },
      returnEmptyString: false,
      returnNull: false,
      saveMissing: true,
      missingKeyHandler: (lngs, ns, key) => {
        if (import.meta.env?.DEV) {
          console.warn(`[i18n] missing key: ${ns}:${key} (lng=${lngs?.[0]})`);
        }
      },
      react: { useSuspense: false },
    });
  return i18n;
}

export function changeLanguage(locale) {
  const next = normalizeLocale(locale);
  return i18n.changeLanguage(next);
}

export function currentLocale() {
  return i18n.language || FALLBACK_LOCALE;
}

export default i18n;
export const t = (key, params) => i18n.t(key, params);
