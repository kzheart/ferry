// 标题风格:AI 重置标题时用的那套规则。读写走引擎 title_style.get/set,
// 「试一试」只在这一页生成预览,不写回任何会话。
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Spinner, ToolIcon, TrashIcon } from "../../shared/ui/icons.jsx";
import {
  DEFAULT_TITLE_STYLE,
  TITLE_EXAMPLE_LIMIT,
  TITLE_LANGUAGES,
  TITLE_MAX_CHARS_RANGE,
  TITLE_PRESETS,
  TITLE_TYPE_LIMIT,
  generateTitles,
  loadTitleStyle,
  sampleSessions,
  saveTitleStyle,
} from "../titles/public.js";
import { Card, GroupTitle, Row, Select, Toggle, inputStyle } from "./parts.jsx";

const TRIAL_COUNT = 5;

function TypeTable({ types, onChange, t }) {
  const update = (index, patch) =>
    onChange(types.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  return (
    <Card>
      {types.map((row, index) => (
        <div key={index} style={{ display: "flex", alignItems: "center", gap: 8,
          padding: "9px 14px", borderTop: index === 0 ? "none" : "1px solid var(--line6)" }}>
          <input value={row.emoji} aria-label={t("settings:titleStyle.typeEmoji")}
            onChange={event => update(index, { emoji: event.target.value })}
            style={{ ...inputStyle, width: 52, textAlign: "center" }} />
          <input value={row.zh} aria-label={t("settings:titleStyle.typeZh")}
            onChange={event => update(index, { zh: event.target.value })}
            style={{ ...inputStyle, flex: 1, minWidth: 0 }} />
          <input value={row.en} aria-label={t("settings:titleStyle.typeEn")}
            onChange={event => update(index, { en: event.target.value })}
            style={{ ...inputStyle, flex: 1, minWidth: 0 }} />
          <button className="ftool-btn" style={{ flex: "none" }}
            title={t("settings:titleStyle.removeType")}
            onClick={() => onChange(types.filter((_, i) => i !== index))}>
            <TrashIcon size={13} />
          </button>
        </div>
      ))}
      <div style={{ padding: "9px 14px", borderTop: "1px solid var(--line6)" }}>
        <button className="fbtn" style={{ height: 28, fontSize: 12 }}
          disabled={types.length >= TITLE_TYPE_LIMIT}
          onClick={() => onChange([...types, { emoji: "", zh: "", en: "" }])}>
          {t("settings:titleStyle.addType")}
        </button>
      </div>
    </Card>
  );
}

function TrialResult({ trial, t }) {
  if (trial.status === "loading") {
    return (
      <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "14px 16px",
        fontSize: 12, color: "var(--tx4)" }}>
        <Spinner size={14} />{t("settings:titleStyle.trialRunning")}
      </div>
    );
  }
  if (trial.status === "error") {
    return (
      <div style={{ padding: "14px 16px", fontSize: 12, color: "var(--err-deep)",
        lineHeight: 1.55 }}>
        {trial.providerMissing
          ? t("settings:titleStyle.trialNoProvider")
          : trial.message}
      </div>
    );
  }
  if (!trial.items.length) {
    return (
      <div style={{ padding: "14px 16px", fontSize: 12, color: "var(--tx4)" }}>
        {t("settings:titleStyle.trialEmpty")}
      </div>
    );
  }
  return (
    <>
      {trial.items.map((item, index) => (
        <div key={`${item.tool}:${item.ref}`} style={{ display: "flex", alignItems: "center",
          gap: 10, padding: "10px 14px",
          borderTop: index === 0 ? "none" : "1px solid var(--line6)" }}>
          <ToolIcon tool={item.tool} size={18} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 11.5, color: "var(--tx4)", whiteSpace: "nowrap",
              overflow: "hidden", textOverflow: "ellipsis" }}>{item.before || "—"}</div>
            <div style={{ fontSize: 12.5, color: item.skip ? "var(--tx4)" : "var(--tx1)",
              marginTop: 2 }}>
              {item.skip ? t("settings:titleStyle.trialSkipped", { reason: item.reason || "" })
                : item.title}
            </div>
          </div>
        </div>
      ))}
    </>
  );
}

export default function TitleStyle({ sessions = [] }) {
  const { t } = useTranslation();
  const [style, setStyle] = useState(DEFAULT_TITLE_STYLE);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(null);
  const [trial, setTrial] = useState(null);

  useEffect(() => {
    let alive = true;
    loadTitleStyle()
      .then(value => { if (alive) setStyle(value); })
      .catch(() => {})
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, []);

  const set = patch => setStyle(current => ({ ...current, ...patch }));
  const examples = [0, 1, 2].map(index => style.examples?.[index] || "");

  const save = async () => {
    setSaving("saving");
    try {
      setStyle(await saveTitleStyle({
        ...style,
        types: (style.types || []).filter(row => row.emoji || row.zh || row.en),
        examples: examples.map(value => value.trim()).filter(Boolean),
      }));
      setSaving("saved");
    } catch (error) {
      setSaving(String(error?.message || error));
    }
  };

  // 试生成用界面上这份(可能还没保存的)风格,预览不写回。
  const tryOut = async () => {
    const picked = sampleSessions(sessions, TRIAL_COUNT);
    if (!picked.length) {
      setTrial({ status: "ready", items: [] });
      return;
    }
    setTrial({ status: "loading", items: [] });
    try {
      const { items } = await generateTitles({
        sessions: picked, includeManual: true, style,
      });
      setTrial({ status: "ready", items });
    } catch (error) {
      setTrial({
        status: "error",
        items: [],
        providerMissing: error?.code === "provider_unavailable",
        message: String(error?.message || error),
      });
    }
  };

  if (loading) {
    return (
      <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "40px 0",
        justifyContent: "center", fontSize: 12, color: "var(--tx4)" }}>
        <Spinner size={14} />{t("settings:titleStyle.loading")}
      </div>
    );
  }

  return (
    <div>
      <GroupTitle first>{t("settings:titleStyle.groupBasic")}</GroupTitle>
      <Card>
        <Row first title={t("settings:titleStyle.preset")}
          desc={t(`settings:titleStyle.presetDesc.${style.preset}`)}>
          <Select value={style.preset} onChange={preset => set({ preset })}>
            {TITLE_PRESETS.map(preset => (
              <option key={preset} value={preset}>
                {t(`settings:titleStyle.presets.${preset}`)}</option>
            ))}
          </Select>
        </Row>
        <Row title={t("settings:titleStyle.language")}>
          <Select value={style.language} onChange={language => set({ language })}>
            {TITLE_LANGUAGES.map(language => (
              <option key={language} value={language}>
                {t(`settings:titleStyle.languages.${language}`)}</option>
            ))}
          </Select>
        </Row>
        <Row title={t("settings:titleStyle.maxChars")} desc={t("settings:titleStyle.maxCharsDesc")}>
          <input type="number" aria-label={t("settings:titleStyle.maxChars")}
            min={TITLE_MAX_CHARS_RANGE[0]} max={TITLE_MAX_CHARS_RANGE[1]}
            value={style.max_chars}
            onChange={event => set({ max_chars: Number(event.target.value) })}
            style={{ ...inputStyle, width: 82 }} />
        </Row>
        <Row title={t("settings:titleStyle.typePrefix")}>
          <Toggle on={style.type_prefix} onChange={value => set({ type_prefix: value })} />
        </Row>
      </Card>

      <GroupTitle>{t("settings:titleStyle.groupTypes")}</GroupTitle>
      <TypeTable types={style.types || []} onChange={types => set({ types })} t={t} />

      <GroupTitle>{t("settings:titleStyle.groupInstructions")}</GroupTitle>
      <Card>
        <div style={{ padding: "12px 14px" }}>
          <textarea value={style.instructions || ""} rows={4}
            aria-label={t("settings:titleStyle.instructions")}
            placeholder={t("settings:titleStyle.instructionsPlaceholder")}
            onChange={event => set({ instructions: event.target.value })}
            style={{ ...inputStyle, height: "auto", width: "100%", boxSizing: "border-box",
              padding: "9px 11px", lineHeight: 1.55, resize: "vertical" }} />
        </div>
      </Card>

      <GroupTitle>{t("settings:titleStyle.groupExamples")}</GroupTitle>
      <Card>
        {examples.map((value, index) => (
          <div key={index} style={{ padding: "9px 14px",
            borderTop: index === 0 ? "none" : "1px solid var(--line6)" }}>
            <input value={value} aria-label={t("settings:titleStyle.example", { n: index + 1 })}
              placeholder={t("settings:titleStyle.examplePlaceholder")}
              onChange={event => set({
                examples: examples
                  .map((old, i) => (i === index ? event.target.value : old))
                  .slice(0, TITLE_EXAMPLE_LIMIT),
              })}
              style={{ ...inputStyle, width: "100%", boxSizing: "border-box" }} />
          </div>
        ))}
      </Card>

      <div style={{ display: "flex", alignItems: "center", gap: 10, margin: "16px 0 0" }}>
        <button className="fbtn-primary" style={{ height: 32, padding: "0 15px", fontSize: 12 }}
          disabled={saving === "saving"} onClick={save}>
          {t("settings:titleStyle.save")}
        </button>
        <button className="fbtn" style={{ height: 32, fontSize: 12 }}
          disabled={trial?.status === "loading"} onClick={tryOut}>
          {t("settings:titleStyle.tryOut")}
        </button>
        {saving && saving !== "saving" && (
          <span style={{ fontSize: 12, color: saving === "saved" ? "var(--ok-deep)" : "var(--err-deep)" }}>
            {saving === "saved" ? t("settings:titleStyle.saved") : saving}
          </span>
        )}
      </div>

      {trial && (
        <>
          <GroupTitle>{t("settings:titleStyle.groupTrial")}</GroupTitle>
          <Card><TrialResult trial={trial} t={t} /></Card>
        </>
      )}
    </div>
  );
}
