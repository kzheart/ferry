import { useTranslation } from "react-i18next";

import {
  TOOL_NAME,
  agentsWithCapability,
} from "../../shared/contracts/tools.js";
import { DangerConfirm } from "../../shared/ui/DangerConfirm.jsx";
import {
  FilterCheckRow,
  FilterPopover,
  FilterRadioRow,
  FilterSectionTitle,
} from "../../shared/ui/FilterPopover.jsx";
import { ToolIcon } from "../../shared/ui/icons.jsx";
import { ACCENT } from "../../shared/ui/toolDisplay.js";
import { summarizePreparedDeletions } from "./sessionDeletionModel.js";

const BROWSABLE_TOOLS = agentsWithCapability("browse");

export function SessionDeleteConfirm({ prepared, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const sess = prepared.session;
  const subCount = (sess.tree_count || 1) - 1;
  const bullets = [
    subCount > 0 && [
      "var(--warn)",
      t("overlays:delete.bulletSub", { n: subCount }),
    ],
    ["var(--err)", t("overlays:delete.bulletIrreversible")],
  ].filter(Boolean);
  return (
    <DangerConfirm
      width={430}
      title={t("overlays:delete.title")}
      desc={t("overlays:delete.desc", {
        title: sess.title || sess.id,
        tool: TOOL_NAME[sess.tool],
      })}
      bullets={bullets}
      confirmLabel={t("overlays:delete.confirmIrreversible")}
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}

export function BatchDeleteConfirm({ prepared, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const summary = summarizePreparedDeletions(prepared);
  const bullets = [
    [
      "var(--err)",
      t("overlays:delete.bulletBatchIrreversible", { n: summary.total }),
    ],
    ["var(--warn)", t("overlays:delete.bulletBatchPartial")],
  ];
  return (
    <DangerConfirm
      width={430}
      title={t("overlays:delete.batchTitle", { n: summary.total })}
      bullets={bullets}
      confirmLabel={t("overlays:delete.confirmBatch")}
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}

// 目录 / 标签的可切换芯片行:单选,再点一次取消
function ChipRow({ items, activeItem, onToggle, mono = false, empty = null }) {
  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
      {items.map(item => {
        const active = activeItem === item;
        return (
          <button
            key={item}
            className={mono ? "mono" : undefined}
            onClick={() => onToggle(item)}
            style={{
              height: 24,
              padding: "0 9px",
              borderRadius: 20,
              border: `1px solid ${active ? ACCENT : "var(--line)"}`,
              background: active ? "var(--acc-soft)" : "var(--surface)",
              color: active ? ACCENT : "var(--tx3)",
              fontSize: 11,
              cursor: "default",
            }}
          >
            {item}
          </button>
        );
      })}
      {empty && items.length === 0 && (
        <span style={{ fontSize: 11, color: "var(--tx5)" }}>{empty}</span>
      )}
    </div>
  );
}

export function LibraryFilter({
  f,
  setF,
  counts,
  dirs,
  tags = [],
  anchor,
  onClose,
  onClear,
}) {
  const { t } = useTranslation();
  const times = [
    ["all", t("overlays:filter.allTime")],
    ["today", t("overlays:filter.today")],
    ["last7", t("overlays:filter.last7")],
    ["last30", t("overlays:filter.last30")],
  ];
  return (
    <FilterPopover anchor={anchor} onClose={onClose} onClear={onClear} t={t}>
      <FilterSectionTitle first>{t("overlays:filter.source")}</FilterSectionTitle>
      {BROWSABLE_TOOLS.map(tool => (
        <FilterCheckRow
          key={tool}
          on={f.src.includes(tool)}
          icon={<ToolIcon tool={tool} size={24} />}
          label={TOOL_NAME[tool]}
          extra={counts[tool] || 0}
          onClick={() => setF(value => ({
            ...value,
            src: value.src.includes(tool)
              ? value.src.filter(item => item !== tool)
              : [...value.src, tool],
          }))}
        />
      ))}
      <FilterSectionTitle>{t("overlays:filter.timeRange")}</FilterSectionTitle>
      {times.map(([key, label]) => (
        <FilterRadioRow
          key={key}
          on={f.time === key}
          label={label}
          onClick={() => setF(value => ({ ...value, time: key }))}
        />
      ))}
      <FilterSectionTitle>{t("overlays:filter.projectDir")}</FilterSectionTitle>
      <ChipRow
        items={dirs}
        mono
        activeItem={f.dir}
        onToggle={dir => setF(value => ({
          ...value,
          dir: value.dir === dir ? null : dir,
        }))}
        empty={t("overlays:filter.noDirs")}
      />
      {tags.length > 0 && (
        <>
          <FilterSectionTitle>{t("overlays:filter.tags")}</FilterSectionTitle>
          <ChipRow
            items={tags}
            activeItem={f.tag}
            onToggle={tag => setF(value => ({
              ...value,
              tag: value.tag === tag ? null : tag,
            }))}
          />
        </>
      )}
      <FilterSectionTitle>{t("overlays:filter.content")}</FilterSectionTitle>
      <FilterCheckRow
        on={f.mig}
        label={t("overlays:filter.onlyMigrated")}
        onClick={() => setF(value => ({ ...value, mig: !value.mig }))}
      />
      <FilterCheckRow
        on={f.sub}
        label={t("overlays:filter.onlySubSessions")}
        onClick={() => setF(value => ({ ...value, sub: !value.sub }))}
      />
    </FilterPopover>
  );
}
