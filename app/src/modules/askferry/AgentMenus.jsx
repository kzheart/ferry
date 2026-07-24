import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  AutoModeIcon,
  Caret,
  CheckIcon,
  ManualModeIcon,
  ProviderIcon,
} from "../../shared/ui/icons.jsx";

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
const MENU_ROW = {
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

export function ModeMenu({ mode, onPick, onClose }) {
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
    <>
      <div
        onMouseDown={onClose}
        style={{ position: "fixed", inset: 0, zIndex: 29 }}
      />
      <div style={{ ...MENU_SHELL, width: 240 }}>
        {options.map(([key, Icon, name, description]) => (
          <div
            key={key}
            className="hov-item"
            onMouseDown={event => {
              event.preventDefault();
              onPick(key);
            }}
            style={{ padding: "7px 9px", borderRadius: 7, cursor: "default" }}
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
          </div>
        ))}
      </div>
    </>
  );
}

export function RoleMenu({ ferry, onClose, onManage }) {
  const { t } = useTranslation();
  return (
    <>
      <div
        onMouseDown={onClose}
        style={{ position: "fixed", inset: 0, zIndex: 29 }}
      />
      <div style={{ ...MENU_SHELL, width: 250 }}>
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
              background: role.id === ferry.selectedRoleId
                ? "var(--acc-soft5)"
                : "transparent",
              fontFamily: "inherit",
              textAlign: "left",
            }}
          >
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

export function ModelMenu({ ferry, health, onClose, onManage }) {
  const { t } = useTranslation();
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
    <>
      <div
        onMouseDown={onClose}
        style={{ position: "fixed", inset: 0, zIndex: 29 }}
      />
      <div style={MENU_SHELL}>
        {panel === "models" ? (
          <>
            <div className="fscroll" style={{ maxHeight: 280, overflowY: "auto" }}>
              {models.map(model => (
                <div
                  key={`${model.provider}/${model.id}`}
                  className="hov-item"
                  onMouseDown={event => {
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
                </div>
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
                <div
                  className="hov-item"
                  onMouseDown={event => {
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
                </div>
              </>
            )}
            <div style={MENU_DIVIDER} />
            <div
              className="hov-item"
              onMouseDown={event => {
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
            </div>
          </>
        ) : (
          <>
            <div
              className="hov-item"
              onMouseDown={event => {
                event.preventDefault();
                setPanel("models");
              }}
              style={MENU_ROW}
            >
              <Caret size={8} dir="left" />
              <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)" }}>
                {t("askferry:model.effort")}
              </span>
            </div>
            <div style={MENU_DIVIDER} />
            {EFFORT_LEVELS.map(level => (
              <div
                key={level}
                className="hov-item"
                onMouseDown={event => {
                  event.preventDefault();
                  pickEffort(level);
                }}
                style={MENU_ROW}
              >
                <span style={{ fontSize: 12.5, color: "var(--tx1)", flex: 1 }}>
                  {t(`askferry:model.effort_${level}`)}
                </span>
                {effort === level && <CheckIcon size={12} />}
              </div>
            ))}
          </>
        )}
      </div>
    </>
  );
}
