// 浏览器 ICU 自带 locale-aware compact notation：
// zh-CN 使用 万/亿，英文使用 K/M/B；无需再维护一套容易漂移的手写阈值。
const finite = value => Number.isFinite(Number(value)) ? Number(value) : 0;

export function formatInteger(value, locale) {
  return new Intl.NumberFormat(locale, {
    maximumFractionDigits: 0,
  }).format(Math.round(finite(value)));
}

export function formatCompactNumber(value, locale, options = {}) {
  return new Intl.NumberFormat(locale, {
    notation: "compact",
    compactDisplay: "short",
    minimumFractionDigits: 0,
    maximumFractionDigits: options.maximumFractionDigits ?? 2,
  }).format(finite(value));
}

export function formatCurrency(value, locale, currency = "USD") {
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency,
    currencyDisplay: "narrowSymbol",
    maximumFractionDigits: 0,
  }).format(finite(value));
}
