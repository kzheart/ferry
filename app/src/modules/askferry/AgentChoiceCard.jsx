import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const statusColor = {
  pending: "var(--accent)",
  answered: "var(--ok)",
  unanswered: "var(--tx5)",
};

export function AgentChoiceCard({ item, onRespond }) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState(item.selected || []);
  const [customText, setCustomText] = useState(item.customText || "");
  const [submitting, setSubmitting] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const open = item.status === "pending";
  // 提交中不卸载按钮区(否则点一下按钮就消失、零反馈),只锁输入
  const pending = open && !submitting;
  const optionValues = useMemo(
    () => new Set((item.options || []).map(option => option?.label).filter(Boolean)),
    [item.options],
  );

  useEffect(() => {
    if (item.status !== "pending") {
      setSelected(item.selected || []);
      setCustomText(item.customText || "");
      // 作答落地即收起,长选项列表答完就不该继续占满整条时间线
      setExpanded(false);
    }
  }, [item.status, item.selected, item.customText]);

  const toggle = value => {
    if (!pending) return;
    setSelected(current => {
      if (!item.multiSelect) return [value];
      return current.includes(value)
        ? current.filter(option => option !== value)
        : [...current, value];
    });
  };

  const canSubmit = selected.length > 0 || (item.allowCustom && customText.trim());
  const respond = async answer => {
    if (submitting) return;
    setSubmitting(true);
    try {
      await onRespond(answer);
    } finally {
      setSubmitting(false);
    }
  };
  const submit = () => canSubmit && respond({
    answered: true,
    selected: selected.filter(value => optionValues.has(value)),
    custom_text: customText.trim(),
  });
  // 「跳过」走 answered:false:契约里这是一等公民,不能只留干等 24 小时这一条路
  const skip = () => respond({ answered: false, selected: [], custom_text: "" });

  const title = item.status === "answered"
    ? t("askferry:choice.answered")
    : item.status === "unanswered"
      ? t("askferry:choice.unanswered")
      : t("askferry:choice.title");

  return (
    <div className="fcard" style={{
      padding: "13px 14px",
      display: "flex",
      flexDirection: "column",
      gap: 10,
      maxWidth: 560,
      borderLeft: `3px solid ${statusColor[item.status] || statusColor.pending}`,
    }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={{
          width: 7,
          height: 7,
          borderRadius: "50%",
          background: statusColor[item.status] || statusColor.pending,
          flex: "none",
        }} />
        <span style={{ fontSize: 12.5, fontWeight: 650, color: "var(--tx1)" }}>
          {title}
        </span>
        {item.multiSelect && (
          <span style={{ fontSize: 11, color: "var(--tx5)" }}>
            {t("askferry:choice.multiSelect")}
          </span>
        )}
      </div>

      <div className="selectable" style={{
        color: "var(--tx2)",
        fontSize: 13,
        lineHeight: 1.55,
        whiteSpace: "pre-wrap",
      }}>
        {item.question}
      </div>

      <fieldset disabled={!pending} style={{
        display: "flex",
        flexDirection: "column",
        gap: 6,
        border: 0,
        padding: 0,
        margin: 0,
      }}>
        <legend style={{
          position: "absolute",
          width: 1,
          height: 1,
          padding: 0,
          margin: -1,
          overflow: "hidden",
          clip: "rect(0, 0, 0, 0)",
          whiteSpace: "nowrap",
          border: 0,
        }}>
          {item.question}
        </legend>
        {(item.options || []).map((option, index) => {
          const value = option?.label || "";
          const checked = selected.includes(value);
          return (
            <label key={`${index}-${value}`} style={{
              display: "flex",
              alignItems: "flex-start",
              gap: 9,
              padding: "8px 10px",
              borderRadius: 8,
              border: `1px solid ${checked ? "var(--accent)" : "var(--line2)"}`,
              background: checked ? "color-mix(in srgb, var(--accent) 9%, transparent)" : "transparent",
              cursor: pending ? "pointer" : "default",
              opacity: option ? 1 : .6,
            }}>
              <input
                type={item.multiSelect ? "checkbox" : "radio"}
                name={`choice-${item.requestId}`}
                checked={checked}
                onChange={() => toggle(value)}
                style={{ marginTop: 2, accentColor: "var(--accent)" }}
              />
              <span style={{ minWidth: 0, flex: 1 }}>
                <span style={{ display: "flex", alignItems: "center", gap: 7,
                  color: "var(--tx1)", fontSize: 12.5, fontWeight: 600 }}>
                  <span>{value}</span>
                  {option?.recommended && (
                    <span style={{ color: "var(--accent)", fontSize: 10.5, fontWeight: 500 }}>
                      {t("askferry:choice.recommended")}
                    </span>
                  )}
                </span>
                {option?.description && (
                  <span className="selectable" style={{ display: "block", marginTop: 2,
                    color: "var(--tx4)", fontSize: 11.5, lineHeight: 1.45 }}>
                    {option.description}
                  </span>
                )}
              </span>
            </label>
          );
        })}
      </fieldset>

      {item.allowCustom && (
        <textarea
          className="selectable"
          value={customText}
          disabled={!pending}
          onChange={event => setCustomText(event.target.value)}
          placeholder={t("askferry:choice.customPlaceholder")}
          rows={2}
          style={{
            resize: "vertical",
            minHeight: 48,
            width: "100%",
            boxSizing: "border-box",
            padding: "8px 9px",
            borderRadius: 7,
            border: "1px solid var(--line2)",
            background: "var(--bg)",
            color: "var(--tx1)",
            font: "inherit",
            fontSize: 12,
          }}
        />
      )}

      {item.status === "unanswered" && (
        <div style={{ color: "var(--tx5)", fontSize: 11.5 }}>
          {t("askferry:choice.noAnswer")}
        </div>
      )}

      {open && (
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 1 }}>
          <button className="fbtn" disabled={submitting} onClick={skip}>
            {t("askferry:choice.skip")}
          </button>
          <button className="fbtn fbtn-primary" disabled={!canSubmit || submitting}
            onClick={submit}>
            {submitting ? t("askferry:choice.submitting") : t("askferry:choice.submit")}
          </button>
        </div>
      )}
    </div>
  );
}
