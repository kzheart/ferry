// 资源栏标题上的两个下拉:范围菜单(导航栏折叠时的替代入口)与显示选项菜单。
// 二者共用一个定位/关闭壳,内容分别是导航栏的范围区和原筛选浮层里"不是范围"的那部分。
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { CheckIcon } from "../shared/ui/icons.jsx";

const MENU_WIDTH = 232;

/** 锚定在触发元素下方的浮层:点外面、Esc、滚动都会关掉它。 */
export function PaneMenu({ anchorRef, width = MENU_WIDTH, align = "left", label,
  onClose, children }) {
  const [pos, setPos] = useState(null);
  const menuRef = useRef(null);

  useEffect(() => {
    const position = () => {
      const rect = anchorRef.current?.getBoundingClientRect();
      if (!rect) return;
      const left = align === "right"
        ? Math.max(8, rect.right - width)
        : Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
      setPos({ top: rect.bottom + 5, left });
    };
    const close = event => {
      if (!anchorRef.current?.contains(event.target)
        && !menuRef.current?.contains(event.target)) onClose();
    };
    const escape = event => { if (event.key === "Escape") onClose(); };
    position();
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", escape);
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", escape);
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
    };
  }, [anchorRef, align, width, onClose]);

  if (!pos) return null;
  return createPortal(
    <div ref={menuRef} role="menu" aria-label={label} className="fscroll"
      style={{ position: "fixed", top: pos.top, left: pos.left, width,
        maxHeight: Math.max(200, window.innerHeight - pos.top - 24), overflowY: "auto",
        zIndex: 70, padding: 5, borderRadius: 8, background: "var(--surface)",
        border: "1px solid var(--line3)", boxShadow: "var(--shadow-menu)" }}>
      {children}
    </div>, document.body,
  );
}

export function MenuHeading({ children }) {
  return (
    <div style={{ height: 22, display: "flex", alignItems: "center", padding: "4px 8px 0",
      fontSize: 11, fontWeight: 600, color: "var(--tx5)", letterSpacing: ".06em",
      textTransform: "uppercase" }}>{children}</div>
  );
}

export function MenuItem({ icon, label, count, checked, selected, title, onClick }) {
  return (
    <button type="button" role="menuitemradio" aria-checked={!!(checked || selected)}
      title={title} onClick={onClick}
      style={{ width: "100%", height: 26, display: "flex", alignItems: "center", gap: 8,
        padding: "0 8px", border: "none", borderRadius: 5, textAlign: "left",
        fontFamily: "inherit", fontSize: 12.5, cursor: "default",
        background: selected ? "var(--acc-soft2)" : "transparent", color: "var(--tx1)" }}
      className="hov-item">
      {icon && <span style={{ flex: "none", display: "flex", color: "var(--tx4b)" }}>{icon}</span>}
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis",
        whiteSpace: "nowrap" }}>{label}</span>
      {checked && <span style={{ flex: "none", display: "flex", color: "var(--tx2)" }}>
        <CheckIcon size={12} /></span>}
      {count != null && (
        <span className="mono" style={{ flex: "none", fontSize: 11, color: "var(--tx5)" }}>
          {count}</span>
      )}
    </button>
  );
}

const Divider = () => (
  <div style={{ height: 1, background: "var(--line3)", margin: "5px 6px" }} />
);
export function DisplayMenu({ anchorRef, display, onChange, onClose }) {
  const { t } = useTranslation();
  const groups = [
    ["project", t("app:display.groupProject")],
    ["time", t("app:display.groupTime")],
    ["none", t("app:display.groupNone")],
  ];
  const times = [
    ["all", t("app:display.timeAll")],
    ["last7", t("app:display.timeLast7")],
    ["last30", t("app:display.timeLast30")],
  ];
  return (
    <PaneMenu anchorRef={anchorRef} align="right" width={208}
      label={t("app:display.menu")} onClose={onClose}>
      <MenuHeading>{t("app:display.group")}</MenuHeading>
      {groups.map(([key, label]) => (
        <MenuItem key={key} label={label} checked={display.group === key}
          onClick={() => onChange({ group: key })} />
      ))}
      <Divider />
      <MenuHeading>{t("app:display.time")}</MenuHeading>
      {times.map(([key, label]) => (
        <MenuItem key={key} label={label} checked={display.time === key}
          onClick={() => onChange({ time: key })} />
      ))}
      <Divider />
      <MenuItem label={t("app:display.onlySub")} checked={display.subOnly}
        onClick={() => onChange({ subOnly: !display.subOnly })} />
      <MenuItem label={t("app:display.onlyMigrated")} checked={display.migOnly}
        onClick={() => onChange({ migOnly: !display.migOnly })} />
      <Divider />
      <MenuItem label={t("app:display.sortUpdated")} checked onClick={() => {}} />
    </PaneMenu>
  );
}
