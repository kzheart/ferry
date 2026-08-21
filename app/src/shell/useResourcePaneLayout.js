import { useEffect, useState } from "react";

import { useDensityMetrics } from "../shared/ui/density.js";

const WIDTH_KEY = "ferry-pane-width";
const COLLAPSED_KEY = "ferry-pane-collapsed";

// 默认宽 / 上下限都跟着密度走(standard 300,240–380;compact 250,190–320)。
// 存量值可能来自旧版本或被手改过,读回时一律钳到当前密度的合法区间。
const readWidth = metrics => {
  const stored = Number(localStorage.getItem(WIDTH_KEY));
  if (!Number.isFinite(stored) || stored <= 0) return metrics.paneDefault;
  return Math.max(metrics.paneMin, Math.min(metrics.paneMax, stored));
};

export function useResourcePaneLayout() {
  const metrics = useDensityMetrics();
  const MIN_WIDTH = metrics.paneMin;
  const MAX_WIDTH = metrics.paneMax;
  const DEFAULT_WIDTH = metrics.paneDefault;
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem(COLLAPSED_KEY) === "1");
  const [width, setWidth] = useState(() => readWidth(metrics));
  const [resizing, setResizing] = useState(false);

  // 切换密度后旧宽度可能落在新区间外(紧凑 190 → 标准最小 240),钳回来
  useEffect(() => {
    setWidth(value => Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, value)));
  }, [MIN_WIDTH, MAX_WIDTH]);

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
