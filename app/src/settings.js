// 外观设置:主题 / 强调色 / 界面字体 / 字号 / 减少动效
// 落 localStorage,并以 CSS 变量与 data-* 属性作用到根节点(组件里统一用 var(--accent))
import { useEffect, useState } from "react";

const KEY = "ferry-settings";

export const ACCENTS = ["#0B67F5", "#0A7EA4", "#5B54E6", "#1C9E5A", "#D06A2C"];

export const FONTS = {
  system: '-apple-system, BlinkMacSystemFont, "PingFang SC", "SF Pro Text", system-ui, sans-serif',
  sans: '"Helvetica Neue", "PingFang SC", system-ui, sans-serif',
  mono: '"JetBrains Mono", ui-monospace, "PingFang SC", monospace',
};

export const FONT_SIZES = [[0.92, "小"], [1, "标准"], [1.08, "大"], [1.18, "特大"]];

export const DEFAULTS = {
  theme: "light", accent: ACCENTS[0], uiFont: "system", fontScale: 1, reduceMotion: false,
};

function load() {
  try { return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) || "{}") }; }
  catch { return { ...DEFAULTS }; }
}

export function useSettings() {
  const [s, setS] = useState(load);
  const [sysDark, setSysDark] = useState(
    () => window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false);

  useEffect(() => {
    const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!mq) return;
    const onChange = () => setSysDark(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const dark = s.theme === "dark" || (s.theme === "system" && sysDark);

  useEffect(() => {
    localStorage.setItem(KEY, JSON.stringify(s));
    const root = document.documentElement;
    root.style.setProperty("--accent", s.accent);
    root.style.setProperty("--font-ui", FONTS[s.uiFont] || FONTS.system);
    root.style.setProperty("--font-size", `${(13 * s.fontScale).toFixed(2)}px`);
    root.dataset.reduce = s.reduceMotion ? "1" : "0";
    root.dataset.theme = dark ? "dark" : "light";
  }, [s, dark]);

  return [s, patch => setS(v => ({ ...v, ...patch })), dark];
}
