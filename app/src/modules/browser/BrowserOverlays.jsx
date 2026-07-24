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
import { ACCENT } from "../../shared/ui/toolDisplay.js";

export function SessionDeleteConfirm({ sess, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const subCount = (sess.tree_count || 1) - 1;
  const isOpenCode = sess.tool === "opencode";
  const bullets = [
    subCount > 0 && [
      "var(--warn)",
      t("overlays:delete.bulletSub", { n: subCount }),
    ],
    ["var(--ok)", t("overlays:delete.bulletSnapshot")],
    isOpenCode
      ? ["var(--err)", t("overlays:delete.bulletOpenCode")]
      : ["var(--accent)", t("overlays:delete.bulletUndoable")],
  ].filter(Boolean);
  return (
    <ConfirmBox
      width={430}
      title={t("overlays:delete.title")}
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
            {isOpenCode
              ? t("overlays:delete.confirmOpenCode")
              : t("overlays:delete.confirmOther")}
          </button>
        </>
      )}
    >
      <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 7, lineHeight: 1.5 }}>
        {t("overlays:delete.desc", {
          title: sess.title || sess.id,
          tool: TOOL_NAME[sess.tool],
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

export function BatchDeleteConfirm({ sessions, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const openCodeCount = sessions
    .filter(session => session.tool === "opencode").length;
  const bullets = [
    ["var(--ok)", t("overlays:delete.bulletBatchSnapshot")],
    openCodeCount > 0 && [
      "var(--err)",
      t("overlays:delete.bulletBatchOpenCode", { n: openCodeCount }),
    ],
    ["var(--accent)", t("overlays:delete.bulletBatchRest")],
  ].filter(Boolean);
  return (
    <ConfirmBox
      width={430}
      title={t("overlays:delete.batchTitle", { n: sessions.length })}
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
            {t("overlays:delete.confirmOther")}
          </button>
        </>
      )}
    >
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
      {TOOLS.map(tool => (
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
      <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
        {dirs.map(dir => {
          const active = f.dir === dir;
          return (
            <button
              key={dir}
              className="mono"
              onClick={() => setF(value => ({
                ...value,
                dir: active ? null : dir,
              }))}
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
              {dir}
            </button>
          );
        })}
        {dirs.length === 0 && (
          <span style={{ fontSize: 11, color: "var(--tx5)" }}>
            {t("overlays:filter.noDirs")}
          </span>
        )}
      </div>
      {tags.length > 0 && (
        <>
          <FilterSectionTitle>{t("overlays:filter.tags")}</FilterSectionTitle>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
            {tags.map(tag => {
              const active = f.tag === tag;
              return (
                <button
                  key={tag}
                  onClick={() => setF(value => ({
                    ...value,
                    tag: active ? null : tag,
                  }))}
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
                  {tag}
                </button>
              );
            })}
          </div>
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
