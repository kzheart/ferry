import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import "./agentChoice.css";

export function AgentChoiceCard({ item, onRespond }) {
  const { t } = useTranslation();
  const questionId = useId();
  const detailsId = useId();
  const cardRef = useRef(null);
  const previousHeight = useRef(null);
  const heightAnimation = useRef(null);
  const [error, setError] = useState("");
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

  useLayoutEffect(() => {
    const card = cardRef.current;
    if (!card) return;
    const from = heightAnimation.current
      ? card.getBoundingClientRect().height
      : previousHeight.current;
    heightAnimation.current?.cancel();
    heightAnimation.current = null;
    const to = card.getBoundingClientRect().height;
    previousHeight.current = to;
    if (!from || from === to || typeof card.animate !== "function"
        || window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return;
    const animation = card.animate([{ height: `${from}px` }, { height: `${to}px` }], {
      duration: 170,
      easing: "ease-out",
    });
    heightAnimation.current = animation;
    animation.onfinish = () => {
      if (heightAnimation.current === animation) heightAnimation.current = null;
    };
  }, [expanded, item.status]);

  useEffect(() => () => heightAnimation.current?.cancel(), []);

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
    setError("");
    try {
      await onRespond(answer);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
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

  // 已作答/未作答默认折叠成一行摘要,点标题行展开看完整选项
  const collapsed = !open && !expanded;
  const summary = [
    ...(item.selected || []),
    ...((item.customText || "").trim() ? [(item.customText || "").trim()] : []),
  ].join(" · ");

  const title = item.status === "answered"
    ? t("askferry:choice.answered")
    : item.status === "unanswered"
      ? t("askferry:choice.unanswered")
      : t("askferry:choice.title");

  const optionContent = option => (
    <span className="agent-choice-option-copy">
      <span className="agent-choice-option-title">
        <span>{option?.label || ""}</span>
        {option?.recommended && (
          <span className="agent-choice-recommended">{t("askferry:choice.recommended")}</span>
        )}
      </span>
      {option?.description && <span className="agent-choice-description">{option.description}</span>}
    </span>
  );

  return (
    <section ref={cardRef} className={`agent-choice ${open ? "is-pending" : "is-complete"}`} aria-labelledby={questionId}>
      {open ? (
        <>
          <div className="agent-choice-eyebrow">
            <span>{title}</span>
            {item.multiSelect && <span>{t("askferry:choice.multiSelect")}</span>}
          </div>
          <div id={questionId} className="agent-choice-question selectable">{item.question}</div>
        </>
      ) : (
        <button className="agent-choice-summary" aria-expanded={expanded} aria-controls={detailsId}
          onClick={() => setExpanded(value => !value)}>
          <span className="agent-choice-status">{title}</span>
          <span className="agent-choice-summary-copy">
            <span id={questionId} className="agent-choice-summary-question">{item.question}</span>
            {summary && <span className="agent-choice-answer">{summary}</span>}
          </span>
          <svg className="agent-choice-chevron" aria-hidden="true" width="14" height="14" viewBox="0 0 16 16">
            <path d="m6 4 4 4-4 4" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
      )}

      {!collapsed && <div id={detailsId} className="agent-choice-details">
        {open ? <>
          <fieldset disabled={!pending} className="agent-choice-options" aria-labelledby={questionId}>
            {(item.options || []).map((option, index) => {
              const value = option?.label || "";
              const checked = selected.includes(value);
              return (
                <label key={`${index}-${value}`} className={`agent-choice-option ${checked ? "is-selected" : ""}`}>
                  <input type={item.multiSelect ? "checkbox" : "radio"}
                    name={`choice-${item.requestId}`} checked={checked} onChange={() => toggle(value)} />
                  {optionContent(option)}
                </label>
              );
            })}
          </fieldset>
          {item.allowCustom && <textarea className="agent-choice-custom selectable" value={customText}
            disabled={!pending} onChange={event => setCustomText(event.target.value)}
            aria-label={t("askferry:choice.customPlaceholder")}
            placeholder={t("askferry:choice.customPlaceholder")} rows={2} />}
          {error && <div className="agent-choice-error" role="alert">{error}</div>}
          <div className="agent-choice-actions">
            <button className="fbtn agent-choice-skip" disabled={submitting} onClick={skip}>
              {t("askferry:choice.skip")}
            </button>
            <button className="fbtn fbtn-primary" disabled={!canSubmit || submitting} onClick={submit}>
              {submitting ? t("askferry:choice.submitting") : t("askferry:choice.submit")}
            </button>
          </div>
        </> : <>
          <ul className="agent-choice-record">
            {(item.options || []).map((option, index) => {
              const checked = (item.selected || []).includes(option?.label);
              return <li key={`${index}-${option?.label}`} className={checked ? "is-selected" : ""}>
                <span className="agent-choice-record-mark" aria-label={checked ? t("askferry:choice.answered") : undefined}>
                  {checked ? "✓" : "·"}
                </span>
                {optionContent(option)}
              </li>;
            })}
          </ul>
          {item.customText?.trim() && <p className="agent-choice-note selectable">{item.customText}</p>}
          {item.status === "unanswered" && <p className="agent-choice-description">{t("askferry:choice.noAnswer")}</p>}
        </>}
      </div>}
    </section>
  );
}
