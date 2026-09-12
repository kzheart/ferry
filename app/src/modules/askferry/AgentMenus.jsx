import { useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";

import {
  AutoModeIcon,
  Caret,
  CheckIcon,
  ManualModeIcon,
  ProviderIcon,
  RoleAvatar,
} from "../../shared/ui/icons.jsx";
import { useFerryRuntime } from "../../shared/capabilities/ferryRuntime.jsx";

const MENU_SHELL = {
  position: "absolute",
  left: 0,
  bottom: "100%",
  marginBottom: 8,
  width: 268,
  background: "var(--bg)",
  borderRadius: 11,
  boxShadow: "var(--shadow-menu)",
  padding: 4,
  zIndex: 30,
  animation: "fpop .14s ease",
};
// 弹层挂在窗口顶层，坐标来自触发按钮，避免聊天滚动区和浮动面板影响定位。
function AgentMenuSurface({ anchorRef, onClose, width, label, children }) {
  const menuRef = useRef(null);
  const [position, setPosition] = useState(null);
  useLayoutEffect(() => {
    const place = () => {
      const anchor = anchorRef.current?.getBoundingClientRect();
      const menu = menuRef.current;
      if (!anchor || !menu) return;
      const viewportWidth = window.innerWidth;
      const viewportHeight = window.innerHeight;
      const above = anchor.top - 8;
      const below = viewportHeight - anchor.bottom - 8;
      const height = menu.getBoundingClientRect().height;
      const left = Math.max(
        8,
        Math.min(
          anchor.left,
          viewportWidth - Math.min(width, viewportWidth - 16) - 8,
        ),
      );
      const preferredTop =
        above >= height || above >= below
          ? anchor.top - height - 8
          : anchor.bottom + 8;
      setPosition({
        left,
        top: Math.max(8, Math.min(preferredTop, viewportHeight - height - 8)),
      });
    };
    place();
    const observer = new ResizeObserver(place);
    if (menuRef.current) observer.observe(menuRef.current);
    if (anchorRef.current) observer.observe(anchorRef.current);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [anchorRef, width]);
  const positioned = position !== null;
  useLayoutEffect(() => {
    if (positioned)
      menuRef.current?.querySelector('[role="menuitem"]')?.focus();
  }, [positioned]);
  const close = () => {
    onClose();
    anchorRef.current?.focus();
  };
  return createPortal(
    <>
      <div
        onMouseDown={close}
        style={{ position: "fixed", inset: 0, zIndex: 60 }}
      />
      <div
        ref={menuRef}
        role="menu"
        aria-label={label}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            close();
            return;
          }
          const items = [
            ...menuRef.current.querySelectorAll('[role="menuitem"]'),
          ];
          const current = items.indexOf(document.activeElement);
          if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
            event.preventDefault();
            const index =
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? items.length - 1
                  : (current +
                      (event.key === "ArrowDown" ? 1 : -1) +
                      items.length) %
                    items.length;
            items[index]?.focus();
          }
        }}
        style={{
          ...MENU_SHELL,
          position: "fixed",
          bottom: "auto",
          marginBottom: 0,
          width,
          maxWidth: "calc(100vw - 16px)",
          maxHeight: "calc(100vh - 16px)",
          overflowY: "auto",
          boxSizing: "border-box",
          zIndex: 61,
          visibility: position ? "visible" : "hidden",
          ...position,
        }}
      >
        {children}
      </div>
    </>,
    document.body,
  );
}

const MENU_BUTTON = {
  width: "100%", border: "none", background: "transparent",
  fontFamily: "inherit", textAlign: "left", color: "inherit",
};
const MENU_ROW = {
  ...MENU_BUTTON,
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "7px 9px",
  borderRadius: 7,
  cursor: "default",
};
const MENU_DIVIDER = {
  height: 1,
  background: "var(--line5)",
  margin: "4px 8px",
};

export function ModeMenu({ anchorRef, mode, onPick, onClose }) {
  const { t } = useTranslation();
  const options = [
    [
      "manual",
      ManualModeIcon,
      t("askferry:mode.manual"),
      t("askferry:mode.manualDesc"),
    ],
    [
      "auto",
      AutoModeIcon,
      t("askferry:mode.auto"),
      t("askferry:mode.autoDesc"),
    ],
  ];
  return (
      <AgentMenuSurface anchorRef={anchorRef} onClose={onClose} width={240} label={t("askferry:mode.manual")}>
        {options.map(([key, Icon, name, description]) => (
          <button type="button" role="menuitem"
            key={key}
            className="hov-item"
            onClick={event => {
              event.preventDefault();
              onPick(key);
            }}
            style={{ ...MENU_BUTTON, padding: "7px 9px", borderRadius: 7, cursor: "default" }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <span style={{
                display: "inline-flex",
                color: key === "auto" ? "var(--warn)" : "var(--tx3b)",
              }}>
                <Icon />
              </span>
              <span style={{
                fontSize: 12.5,
                fontWeight: 600,
                color: "var(--tx1)",
                flex: 1,
              }}>
                {name}
              </span>
              {mode === key && <CheckIcon size={12} />}
            </div>
            <div style={{
              fontSize: 11,
              color: "var(--tx4)",
              lineHeight: 1.45,
              marginTop: 2,
            }}>
              {description}
            </div>
          </button>
        ))}
      </AgentMenuSurface>
  );
}

// 角色胶囊的下拉:列表可滚动,角色多时不撑爆版面;menuStyle 用于覆盖弹出方位
export function RoleMenu({ onClose, onManage, menuStyle }) {
  const { t } = useTranslation();
  const ferry = useFerryRuntime();
  return (
    <>
      <div
        onMouseDown={onClose}
        style={{ position: "fixed", inset: 0, zIndex: 29 }}
      />
      <div style={{ ...MENU_SHELL, width: 250, ...menuStyle }}>
        <div className="fscroll" style={{ maxHeight: 280, overflowY: "auto" }}>
          {(ferry.roles || []).map(role => (
            <button
              key={role.id}
              type="button"
              className="hov-item"
              onMouseDown={event => {
                event.preventDefault();
                ferry.setSelectedRoleId(role.id);
                onClose();
              }}
              style={{
                ...MENU_ROW,
                width: "100%",
                border: "none",
                background: "transparent",
                fontFamily: "inherit",
                textAlign: "left",
              }}
            >
              <RoleAvatar icon={role.icon} color={role.color} size={24} />
              <span style={{ flex: 1, minWidth: 0 }}>
                <span style={{
                  display: "block",
                  fontSize: 12.5,
                  fontWeight: 600,
                  color: "var(--tx1)",
                }}>
                  {role.name}
                </span>
                <span style={{
                  display: "block",
                  fontSize: 10.5,
                  color: "var(--tx5)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}>
                  {role.description || role.tools?.join(" · ")}
                </span>
              </span>
              {role.id === ferry.selectedRoleId && <CheckIcon size={12} />}
            </button>
          ))}
        </div>
        <div style={MENU_DIVIDER} />
        <button
          type="button"
          className="hov-item"
          onMouseDown={event => {
            event.preventDefault();
            onManage();
            onClose();
          }}
          style={{
            ...MENU_ROW,
            width: "100%",
            border: "none",
            background: "transparent",
            fontFamily: "inherit",
            fontSize: 12,
            color: "var(--tx2)",
          }}
        >
          {t("askferry:role.manage")}
        </button>
      </div>
    </>
  );
}

const EFFORT_LEVELS = ["off", "low", "medium", "high"];

export function ModelMenu({ anchorRef, health, onClose, onManage }) {
  const { t } = useTranslation();
  const ferry = useFerryRuntime();
  const [panel, setPanel] = useState("models");
  const models = ferry.models || [];
  const current = models.find(model =>
    model.provider === health?.provider && model.id === health?.model);
  const effort = health?.thinking || "off";

  const pick = model => {
    onClose();
    ferry.selectModel(
      model.provider,
      model.id,
      model.reasoning ? effort : undefined,
    ).catch(ferry.reportError);
  };
  const pickEffort = level => {
    onClose();
    if (current) {
      ferry.selectModel(current.provider, current.id, level)
        .catch(ferry.reportError);
    }
  };

  return (
      <AgentMenuSurface anchorRef={anchorRef} onClose={onClose} width={268} label={t("askferry:model.pick")}>
        {panel === "models" ? (
          <>
            <div className="fscroll" style={{ maxHeight: 280, overflowY: "auto" }}>
              {models.map(model => (
                <button type="button" role="menuitem"
                  key={`${model.provider}/${model.id}`}
                  className="hov-item"
                  onClick={event => {
                    event.preventDefault();
                    pick(model);
                  }}
                  style={{ ...MENU_ROW, alignItems: "flex-start" }}
                >
                  <ProviderIcon provider={model.provider} size={15} />
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{
                      fontSize: 12.5,
                      fontWeight: 600,
                      color: "var(--tx1)",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}>
                      {model.name}
                    </div>
                    <div style={{ fontSize: 11, color: "var(--tx4)", marginTop: 1 }}>
                      {model.provider_name}
                      {model.reasoning
                        ? ` · ${t("askferry:model.reasoning")}`
                        : ""}
                    </div>
                  </div>
                  {current === model && <CheckIcon size={12} />}
                </button>
              ))}
              {!models.length && (
                <div style={{
                  fontSize: 11.5,
                  color: "var(--tx5)",
                  padding: "12px 9px",
                  lineHeight: 1.55,
                }}>
                  {t("askferry:model.empty")}
                </div>
              )}
            </div>
            {current?.reasoning && (
              <>
                <div style={MENU_DIVIDER} />
                <button type="button" role="menuitem"
                  className="hov-item"
                  onClick={event => {
                    event.preventDefault();
                    setPanel("effort");
                  }}
                  style={MENU_ROW}
                >
                  <span style={{
                    fontSize: 12.5,
                    fontWeight: 600,
                    color: "var(--tx1)",
                    flex: 1,
                  }}>
                    {t("askferry:model.effort")}
                  </span>
                  <span style={{ fontSize: 12, color: "var(--tx4)" }}>
                    {t(`askferry:model.effort_${effort}`)}
                  </span>
                  <Caret size={8} dir="right" />
                </button>
              </>
            )}
            <div style={MENU_DIVIDER} />
            <button type="button" role="menuitem"
              className="hov-item"
              onClick={event => {
                event.preventDefault();
                onClose();
                onManage();
              }}
              style={MENU_ROW}
            >
              <span style={{
                fontSize: 12.5,
                fontWeight: 600,
                color: "var(--tx1)",
                flex: 1,
              }}>
                {t("askferry:model.manage")}
              </span>
              <Caret size={8} dir="right" />
            </button>
          </>
        ) : (
          <>
            <button type="button" role="menuitem"
              className="hov-item"
              onClick={event => {
                event.preventDefault();
                setPanel("models");
              }}
              style={MENU_ROW}
            >
              <Caret size={8} dir="left" />
              <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)" }}>
                {t("askferry:model.effort")}
              </span>
            </button>
            <div style={MENU_DIVIDER} />
            {EFFORT_LEVELS.map(level => (
              <button type="button" role="menuitem"
                key={level}
                className="hov-item"
                onClick={event => {
                  event.preventDefault();
                  pickEffort(level);
                }}
                style={MENU_ROW}
              >
                <span style={{ fontSize: 12.5, color: "var(--tx1)", flex: 1 }}>
                  {t(`askferry:model.effort_${level}`)}
                </span>
                {effort === level && <CheckIcon size={12} />}
              </button>
            ))}
          </>
        )}
      </AgentMenuSurface>
  );
}
