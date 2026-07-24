import { useCallback, useEffect, useRef, useState } from "react";

export const DEFAULT_RAIL_ORDER = ["overview", "askferry", "library", "history"];

function isKnownRailKey(key) {
  return DEFAULT_RAIL_ORDER.includes(key);
}

export function normalizeRailOrder(value) {
  if (!Array.isArray(value)) return [...DEFAULT_RAIL_ORDER];
  const order = value.filter((key, index) => isKnownRailKey(key) && value.indexOf(key) === index);
  return [...order, ...DEFAULT_RAIL_ORDER.filter(key => !order.includes(key))];
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
    return normalizeRailOrder(JSON.parse(localStorage.getItem(storageKey) || "null"));
  } catch {
    return [...DEFAULT_RAIL_ORDER];
  }
}

export function useRailNavigation({ labels, storageKey }) {
  const [railOrder, setRailOrder] = useState(() => loadRailOrder(storageKey));
  const [railTip, setRailTip] = useState(null);
  const [draggingKey, setDraggingKey] = useState(null);
  const [dropTarget, setDropTarget] = useState(null);
  const tipTimer = useRef(null);
  const pointer = useRef(null);
  const suppressClick = useRef(false);

  const leave = useCallback(() => {
    clearTimeout(tipTimer.current);
    setRailTip(null);
  }, []);

  const enter = useCallback((label, event) => {
    const item = event.currentTarget.getBoundingClientRect();
    const root = document.querySelector("[data-ferry-win]");
    if (!root) return;
    const top = item.top - root.getBoundingClientRect().top + item.height / 2;
    clearTimeout(tipTimer.current);
    tipTimer.current = setTimeout(() => setRailTip({ label, top }), 450);
  }, []);

  useEffect(() => () => clearTimeout(tipTimer.current), []);

  const dropAt = useCallback((x, y) => {
    const target = document.elementFromPoint(x, y)?.closest?.("[data-rail-key]");
    const key = target?.dataset.railKey;
    if (!isKnownRailKey(key)) return null;
    const rect = target.getBoundingClientRect();
    return { key, position: y < rect.top + rect.height / 2 ? "before" : "after" };
  }, []);

  const reorder = useCallback((source, target, position) => {
    setRailOrder(order => {
      const next = reorderRailOrder(order, source, target, position);
      if (next === order) return order;
      try {
        localStorage.setItem(storageKey, JSON.stringify(next));
      } catch {
        // 存储不可用时，保持本次会话中的排序结果。
      }
      return next;
    });
  }, [storageKey]);

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
      leave();
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
    railTip,
    draggingKey,
    dropTarget,
    enter,
    leave,
    shouldSuppressClick: () => suppressClick.current,
    pointerHandlers: {
      down: onPointerDown,
      move: onPointerMove,
      up: onPointerUp,
      cancel: onPointerCancel,
    },
  };
}
