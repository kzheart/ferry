// 外观设置:主题 / 减少动效
// 落 localStorage,并以 data-* 属性作用到根节点;配色为黑白中性色,由 style.css 变量定义
import { useEffect, useState } from "react";

const KEY = "ferry-settings";

export const DEFAULTS = {
  theme: "light",
  reduceMotion: false,
  runtimeProbe: false,
  autoCheckUpdates: true,
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
  }, [s, dark]);

  return [s, patch => setS(v => ({ ...v, ...patch })), dark];
}
