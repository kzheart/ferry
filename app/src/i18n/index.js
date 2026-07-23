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
import zhCNAskFerry from "../locales/zh-CN/askferry.json";
import enAskFerry from "../locales/en/askferry.json";
import zhCNOrganizing from "../locales/zh-CN/organizing.json";
import enOrganizing from "../locales/en/organizing.json";

// 新增语言只需在这里加一行(外加 RESOURCES 注册),设置页下拉框自动出现该选项。
// nativeName 用目标语言自称,不做翻译——找母语的人认自己的语言名最快。
export const LOCALE_META = [
  { code: "zh-CN", nativeName: "简体中文" },
  { code: "en", nativeName: "English" },
];

export const SUPPORTED_LOCALES = LOCALE_META.map(l => l.code);
export const FALLBACK_LOCALE = "zh-CN";
export const DEFAULT_LOCALE = null;

const RESOURCES = {
  "zh-CN": { common: zhCNCommon, errors: zhCNErrors, events: zhCNEvents, browser: zhCNBrowser, migration: zhCNMigration, onboarding: zhCNOnboarding, settings: zhCNSettings, overlays: zhCNOverlays, app: zhCNApp, overview: zhCNOverview, askferry: zhCNAskFerry, organizing: zhCNOrganizing },
  "en": { common: enCommon, errors: enErrors, events: enEvents, browser: enBrowser, migration: enMigration, onboarding: enOnboarding, settings: enSettings, overlays: enOverlays, app: enApp, overview: enOverview, askferry: enAskFerry, organizing: enOrganizing },
};

const SETTINGS_KEY = "ferry-settings";

function readStoredLocale() {
  try {
    const s = JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}");
    return s.locale ?? null;
  } catch { return null; }
}

// "zh-CN" -> "zh":按语言主标签匹配,地区变体(zh-TW/en-GB)落到同语言的受支持项
function matchByLanguageTag(tag) {
  const lc = String(tag || "").toLowerCase();
  if (!lc) return null;
  const exact = SUPPORTED_LOCALES.find(c => c.toLowerCase() === lc);
  if (exact) return exact;
  const primary = lc.split("-")[0];
  return SUPPORTED_LOCALES.find(c => c.toLowerCase().split("-")[0] === primary) || null;
}

export function matchSystemLocale() {
  return matchByLanguageTag(navigator.language) || FALLBACK_LOCALE;
}

export function resolveLocale() {
  const stored = readStoredLocale();
  return stored || matchSystemLocale();
}

export function normalizeLocale(locale) {
  if (!locale) return matchSystemLocale();
  return matchByLanguageTag(locale) || FALLBACK_LOCALE;
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
      ns: ["common", "errors", "events", "browser", "migration", "onboarding", "settings", "overlays", "app", "overview", "askferry", "organizing"],
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
  // 日期按当前语言本地化,不手写月份/星期译文——新增语言时自动跟随。
  // 注意:必须用 formatter API,i18next v21 起 interpolation.format 已不再生效。
  i18n.services.formatter.add("monthDay", (value, lng) =>
    value instanceof Date
      ? value.toLocaleDateString(lng, { month: "short", day: "numeric" })
      : value);
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
