// 外观设置:主题 / 减少动效 / 语言
// 落 localStorage,并以 data-* 属性作用到根节点;配色为黑白中性色,由 style.css 变量定义
import { useEffect, useState } from "react";
import { setWindowTheme } from "../../platform/desktop/client.js";
import { changeLanguage } from "../../i18n/index.js";

const KEY = "ferry-settings";

export const DEFAULTS = {
  theme: "light",
  reduceMotion: false,
  runtimeProbe: false,
  terminalApp: "auto",
  autoCheckUpdates: true,
  locale: null,
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
    root.dataset.reduce = s.reduceMotion ? "1" : "0";
    root.dataset.theme = dark ? "dark" : "light";
    // 窗口外观(毛玻璃材质/红绿灯)必须与应用主题同步,
    // 否则深色 CSS 叠在浅色 NSVisualEffectView 上会发灰;跟随系统时交还系统决定
    setWindowTheme(s.theme === "system" ? null : s.theme).catch(() => {});
  }, [s, dark]);

  // locale 变化时同步到 i18next(跟随系统时传 null,由 changeLanguage 归一化)
  useEffect(() => {
    changeLanguage(s.locale);
  }, [s.locale]);

  return [s, patch => setS(v => ({ ...v, ...patch })), dark];
}
