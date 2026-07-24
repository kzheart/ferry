import { useEffect, useRef } from "react";
import {
  onMenu,
  preloadWindow,
  startWindowDrag,
  toggleWindowMaximize,
} from "../platform/desktop/client.js";

export function useDesktopChrome({ onOpenSettings, onToggleSidebar, onRescan }) {
  const menuActions = useRef({});
  menuActions.current = {
    settings: onOpenSettings,
    "toggle-sidebar": onToggleSidebar,
    rescan: onRescan,
  };

  useEffect(() => {
    let unlisten;
    onMenu(id => menuActions.current[id]?.()).then(value => { unlisten = value; });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    preloadWindow();
    const onMouseDown = event => {
      if (event.button !== 0) return;
      const target = event.target;
      if (!target?.hasAttribute?.("data-tauri-drag-region")) return;
      if (event.detail === 2) toggleWindowMaximize();
      else startWindowDrag();
    };
    window.addEventListener("mousedown", onMouseDown);
    return () => window.removeEventListener("mousedown", onMouseDown);
  }, []);
}
