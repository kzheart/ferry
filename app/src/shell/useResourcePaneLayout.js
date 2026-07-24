import { useState } from "react";

const DEFAULT_WIDTH = 232;
const MIN_WIDTH = 190;
const MAX_WIDTH = 360;

export function useResourcePaneLayout() {
  const [collapsed, setCollapsed] = useState(false);
  const [width, setWidth] = useState(DEFAULT_WIDTH);
  const [resizing, setResizing] = useState(false);

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
