import { useTranslation } from "react-i18next";

import { TOOL_NAME } from "../../shared/contracts/tools.js";
import { DangerConfirm } from "../../shared/ui/DangerConfirm.jsx";
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
        tool: TOOL_NAME[history.dst] || history.dst,
      }),
    ],
    ["var(--err)", t("overlays:historyDelete.bulletIrreversible")],
  ];
  return (
    <DangerConfirm
      width={420}
      title={t("overlays:historyDelete.title")}
      desc={t("overlays:historyDelete.desc", {
        title: history.title || history.source_id,
      })}
      bullets={bullets}
      confirmLabel={t("overlays:historyDelete.confirm")}
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}

export function HistoryFilter({ f, setF, tools, anchor, onClose, onClear }) {
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
      {tools.map(tool => (
        <FilterCheckRow
          key={tool}
          on={f.src.includes(tool)}
          icon={<ToolIcon tool={tool} size={24} />}
          label={TOOL_NAME[tool] || tool}
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
        ...tools.map(tool => [tool, TOOL_NAME[tool] || tool]),
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
