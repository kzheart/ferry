import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import zhCNCommon from "../locales/zh-CN/common.json";
import enCommon from "../locales/en/common.json";

export const SUPPORTED_LOCALES = ["zh-CN", "en"];
export const FALLBACK_LOCALE = "zh-CN";
export const DEFAULT_LOCALE = null;

const RESOURCES = {
  "zh-CN": { common: zhCNCommon },
  "en": { common: enCommon },
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
      ns: ["common"],
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
