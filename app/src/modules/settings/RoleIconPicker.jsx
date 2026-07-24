// 角色头像的图标与配色选择器。
// 详情区是滚动容器,弹层必须走 portal + fixed,否则会被 overflow 裁掉。
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  ROLE_COLORS, ROLE_ICON_GROUPS, roleColorVar, roleIconPath,
} from "../../shared/ui/roleIcons.js";

export default function RoleIconPicker({ anchorRef, value, color, onPick, onClose }) {
  const { t } = useTranslation();
  const [position, setPosition] = useState(null);
  useEffect(() => {
    const place = () => {
      const rect = anchorRef.current?.getBoundingClientRect();
      if (!rect) return;
      const top = Math.min(rect.bottom + 6, window.innerHeight - 372);
      setPosition({ top: Math.max(12, top), left: rect.left });
    };
    const escape = event => { if (event.key === "Escape") onClose(); };
    place();
    document.addEventListener("keydown", escape);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      document.removeEventListener("keydown", escape);
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [anchorRef, onClose]);
  if (!position) return null;
  return createPortal(
    <>
      <div onMouseDown={onClose} style={{ position: "fixed", inset: 0, zIndex: 69 }} />
      <div style={{ position: "fixed", top: position.top, left: position.left, width: 268,
        background: "var(--surface)", borderRadius: 12, boxShadow: "var(--shadow-menu)",
        padding: 10, zIndex: 70, animation: "fpop .14s ease" }}>
        <div style={{ fontSize: 10.5, fontWeight: 700, color: "var(--tx5)",
          letterSpacing: ".05em", marginBottom: 7 }}>{t("settings:roles.colorPickerTitle")}</div>
        <div style={{ display: "flex", gap: 6, marginBottom: 11 }}>
          {ROLE_COLORS.map(name => (
            <button key={name} type="button" onClick={() => onPick({ color: name })}
              aria-label={name} aria-pressed={name === color}
              style={{ width: 22, height: 22, borderRadius: "50%", flex: "none", cursor: "default",
                background: roleColorVar(name), padding: 0,
                border: name === color
                  ? "2px solid var(--tx1)" : "2px solid transparent",
                boxShadow: name === color ? "0 0 0 1px var(--surface) inset" : "none" }} />
          ))}
        </div>
        <div className="fscroll" style={{ maxHeight: 246, overflowY: "auto" }}>
          {ROLE_ICON_GROUPS.map(group => (
            <div key={group.key} style={{ marginBottom: 8 }}>
              <div style={{ fontSize: 10.5, fontWeight: 700, color: "var(--tx5)",
                letterSpacing: ".05em", margin: "0 0 5px 2px" }}>
                {t(`settings:roles.iconGroup.${group.key}`)}</div>
              <div style={{ display: "grid", gridTemplateColumns: "repeat(6, 1fr)", gap: 3 }}>
                {group.names.map(name => {
                  const on = name === value;
                  return (
                    <button key={name} type="button" title={name}
                      className={on ? undefined : "hov-item"}
                      onClick={() => onPick({ icon: name })}
                      style={{ height: 32, border: "none", borderRadius: 8, cursor: "default",
                        display: "inline-flex", alignItems: "center", justifyContent: "center",
                        background: on ? "var(--seg-on)" : "transparent",
                        boxShadow: on ? "0 0 0 1px var(--acc-line)" : "none",
                        color: on ? roleColorVar(color) : "var(--tx3b)" }}>
                      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
                        strokeLinecap="round" strokeLinejoin="round" aria-hidden
                        style={{ width: 16, height: 16 }}
                        dangerouslySetInnerHTML={{ __html: roleIconPath(name) }} />
                    </button>
                  );
                })}
              </div>
            </div>
          ))}
        </div>
      </div>
    </>,
    document.body,
  );
}
