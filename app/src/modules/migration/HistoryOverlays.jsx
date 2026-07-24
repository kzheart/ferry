import { useTranslation } from "react-i18next";

import { TOOL_NAME, TOOLS } from "../../shared/contracts/tools.js";
import { ConfirmBox } from "../../shared/ui/ConfirmBox.jsx";
import {
  FilterCheckRow,
  FilterPopover,
  FilterRadioRow,
  FilterSectionTitle,
} from "../../shared/ui/FilterPopover.jsx";
import { ToolIcon } from "../../shared/ui/icons.jsx";
import { STATUS_CODE } from "./migrationModel.js";

export function HistoryDeleteConfirm({ history, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const bullets = [
    [
      "var(--ok)",
      t("overlays:historyDelete.bulletTarget", {
        tool: TOOL_NAME[history.dst],
      }),
    ],
    ["var(--err)", t("overlays:historyDelete.bulletIrreversible")],
  ];
  return (
    <ConfirmBox
      width={420}
      title={t("overlays:historyDelete.title")}
      actions={(
        <>
          <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>
            {t("overlays:delete.cancel")}
          </button>
          <button
            style={{
              height: 34,
              padding: "0 16px",
              background: "var(--err2)",
              border: "none",
              borderRadius: 8,
              fontSize: 13,
              color: "#fff",
              cursor: "default",
              fontWeight: 600,
            }}
            onClick={onConfirm}
          >
            {t("overlays:historyDelete.confirm")}
          </button>
        </>
      )}
    >
      <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 7, lineHeight: 1.5 }}>
        {t("overlays:historyDelete.desc", {
          title: history.title || history.source_id,
        })}
      </div>
      <div style={{
        marginTop: 14,
        border: "1px solid var(--line3)",
        borderRadius: 10,
        padding: "12px 14px",
        display: "flex",
        flexDirection: "column",
        gap: 9,
      }}>
        {bullets.map(([color, text], index) => (
          <div
            key={index}
            style={{
              display: "flex",
              gap: 9,
              fontSize: 12,
              color: "var(--tx2b)",
              lineHeight: 1.45,
            }}
          >
            <span style={{
              width: 5,
              height: 5,
              borderRadius: "50%",
              background: color,
              flex: "none",
              marginTop: 6,
            }} />
            {text}
          </div>
        ))}
      </div>
    </ConfirmBox>
  );
}

export function HistoryFilter({ f, setF, anchor, onClose, onClear }) {
  const { t } = useTranslation();
  const statusOptions = [
    [STATUS_CODE.success, t(`common:${STATUS_CODE.success}`)],
    [STATUS_CODE.failed, t(`common:${STATUS_CODE.failed}`)],
    [STATUS_CODE.rolledBack, t(`common:${STATUS_CODE.rolledBack}`)],
  ];
  const timeOptions = [
    ["all", t("overlays:filter.allTime")],
    ["today", t("overlays:filter.today")],
    ["yesterday", t("overlays:filter.yesterday")],
    ["earlier", t("overlays:filter.earlier")],
  ];
  return (
    <FilterPopover anchor={anchor} onClose={onClose} onClear={onClear} t={t}>
      <FilterSectionTitle first>
        {t("overlays:filter.sourceTools")}
      </FilterSectionTitle>
      {TOOLS.map(tool => (
        <FilterCheckRow
          key={tool}
          on={f.src.includes(tool)}
          icon={<ToolIcon tool={tool} size={24} />}
          label={TOOL_NAME[tool]}
          onClick={() => setF(value => ({
            ...value,
            src: value.src.includes(tool)
              ? value.src.filter(item => item !== tool)
              : [...value.src, tool],
          }))}
        />
      ))}
      <FilterSectionTitle>
        {t("overlays:filter.targetTool")}
      </FilterSectionTitle>
      {[
        ["all", t("overlays:filter.allTargets")],
        ...TOOLS.map(tool => [tool, TOOL_NAME[tool]]),
      ].map(([key, label]) => (
        <FilterRadioRow
          key={key}
          on={f.target === key}
          label={label}
          onClick={() => setF(value => ({ ...value, target: key }))}
        />
      ))}
      <FilterSectionTitle>{t("overlays:filter.status")}</FilterSectionTitle>
      <FilterRadioRow
        on={f.status === "all"}
        label={t("common:status.all")}
        onClick={() => setF(value => ({ ...value, status: "all" }))}
      />
      {statusOptions.map(([key, label]) => (
        <FilterRadioRow
          key={key}
          on={f.status === key}
          label={label}
          onClick={() => setF(value => ({ ...value, status: key }))}
        />
      ))}
      <FilterSectionTitle>
        {t("overlays:filter.timeRange")}
      </FilterSectionTitle>
      {timeOptions.map(([key, label]) => (
        <FilterRadioRow
          key={key}
          on={f.time === key}
          label={label}
          onClick={() => setF(value => ({ ...value, time: key }))}
        />
      ))}
    </FilterPopover>
  );
}
