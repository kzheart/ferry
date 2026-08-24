import { useCallback, useMemo, useRef, useState } from "react";

import { filterByFeatures } from "../shared/capabilities/features.jsx";

// 导航轨的键表。标了 feature 的项由对应开关决定存不存在:开关关着时这条工作区
// 整个不存在——顺序表、用户存在 localStorage 里的自定义顺序、拖拽落点判定,三处
// 都按同一份可用键表过滤。
export const RAIL_ITEMS = [
  { key: "overview" },
  { key: "askferry", feature: "builtin-agent" },
  { key: "library" },
];

export const DEFAULT_RAIL_ORDER = RAIL_ITEMS.map(item => item.key);

// 缺省一律「关」:漏传判定函数时宁可少显示一个入口。
const NOTHING_ENABLED = () => false;

export function railKeys(isFeatureEnabled = NOTHING_ENABLED) {
  return filterByFeatures(RAIL_ITEMS, isFeatureEnabled).map(item => item.key);
}

export function normalizeRailOrder(value, isFeatureEnabled = NOTHING_ENABLED) {
  const known = railKeys(isFeatureEnabled);
  if (!Array.isArray(value)) return known;
  const order = value.filter((key, index) => known.includes(key) && value.indexOf(key) === index);
  return [...order, ...known.filter(key => !order.includes(key))];
}

export function reorderRailOrder(order, source, target, position) {
  if (!source || !target || source === target) return order;
  const next = order.filter(key => key !== source);
  const targetIndex = next.indexOf(target);
  if (targetIndex < 0) return order;
  const index = targetIndex + (position === "after" ? 1 : 0);
  next.splice(index, 0, source);
  return next;
}

function loadRailOrder(storageKey) {
  try {
    return JSON.parse(localStorage.getItem(storageKey) || "null");
  } catch {
    return null;
  }
}

export function useRailNavigation({ labels, storageKey, isFeatureEnabled }) {
  const [storedOrder, setStoredOrder] = useState(() => loadRailOrder(storageKey));
  // 开关一变可用键表就变,顺序当场重算:被开关挡住的能力都是懒启动的,入口不必等重启
  const railOrder = useMemo(
    () => normalizeRailOrder(storedOrder, isFeatureEnabled),
    [storedOrder, isFeatureEnabled],
  );
  const [draggingKey, setDraggingKey] = useState(null);
  const [dropTarget, setDropTarget] = useState(null);
  const pointer = useRef(null);
  const suppressClick = useRef(false);

  const dropAt = useCallback((x, y) => {
    const target = document.elementFromPoint(x, y)?.closest?.("[data-rail-key]");
    const key = target?.dataset.railKey;
    if (!railOrder.includes(key)) return null;
    const rect = target.getBoundingClientRect();
    return { key, position: y < rect.top + rect.height / 2 ? "before" : "after" };
  }, [railOrder]);

  const reorder = useCallback((source, target, position) => {
    const next = reorderRailOrder(railOrder, source, target, position);
    if (next === railOrder) return;
    setStoredOrder(next);
    try {
      localStorage.setItem(storageKey, JSON.stringify(next));
    } catch {
      // 存储不可用时，保持本次会话中的排序结果。
    }
  }, [railOrder, storageKey]);

  const onPointerDown = event => {
    if (event.button !== 0 || event.isPrimary === false) return;
    pointer.current = {
      key: event.currentTarget.dataset.railKey,
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      dragging: false,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  };

  const onPointerMove = event => {
    const drag = pointer.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (!drag.dragging) {
      if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 5) return;
      drag.dragging = true;
      setDraggingKey(drag.key);
    }
    event.preventDefault();
    setDropTarget(dropAt(event.clientX, event.clientY));
  };

  const onPointerUp = event => {
    const drag = pointer.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (drag.dragging) {
      const drop = dropAt(event.clientX, event.clientY);
      if (drop) reorder(drag.key, drop.key, drop.position);
      suppressClick.current = true;
      window.setTimeout(() => { suppressClick.current = false; }, 0);
    }
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    pointer.current = null;
    setDraggingKey(null);
    setDropTarget(null);
  };

  const onPointerCancel = event => {
    if (pointer.current?.pointerId !== event.pointerId) return;
    pointer.current = null;
    setDraggingKey(null);
    setDropTarget(null);
  };

  return {
    items: railOrder.map(key => ({ key, label: labels[key] })).filter(item => item.label),
    draggingKey,
    dropTarget,
    shouldSuppressClick: () => suppressClick.current,
    pointerHandlers: {
      down: onPointerDown,
      move: onPointerMove,
      up: onPointerUp,
      cancel: onPointerCancel,
    },
  };
}
