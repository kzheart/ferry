import { useEffect, useState } from "react";

const DEFAULT_WIDTH = 232;
const MIN_WIDTH = 190;
const MAX_WIDTH = 360;
const WIDTH_KEY = "ferry-pane-width";
const COLLAPSED_KEY = "ferry-pane-collapsed";

// 存量值可能来自旧版本或被手改过,读回时一律钳到合法区间
const readWidth = () => {
  const stored = Number(localStorage.getItem(WIDTH_KEY));
  if (!Number.isFinite(stored) || stored <= 0) return DEFAULT_WIDTH;
  return Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, stored));
};

export function useResourcePaneLayout() {
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem(COLLAPSED_KEY) === "1");
  const [width, setWidth] = useState(readWidth);
  const [resizing, setResizing] = useState(false);

  // 拖动过程中每帧写 localStorage 太重,拖完(resizing 落回 false)再落盘
  useEffect(() => {
    if (!resizing) localStorage.setItem(WIDTH_KEY, String(width));
  }, [width, resizing]);
  useEffect(() => {
    localStorage.setItem(COLLAPSED_KEY, collapsed ? "1" : "0");
  }, [collapsed]);

  const startResize = event => {
    if (collapsed) return;
    const startX = event.clientX;
    const startWidth = width;
    const move = pointer => {
      const next = startWidth + pointer.clientX - startX;
      setWidth(Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, next)));
    };
    const finish = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", finish);
      setResizing(false);
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", finish);
    setResizing(true);
    event.preventDefault();
  };

  return {
    collapsed,
    width,
    resizing,
    startResize,
    resetWidth: () => setWidth(DEFAULT_WIDTH),
    toggleCollapsed: () => setCollapsed(value => !value),
  };
}
