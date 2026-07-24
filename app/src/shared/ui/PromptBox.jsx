import { useState } from "react";
import { useTranslation } from "react-i18next";

import { ConfirmBox } from "./ConfirmBox.jsx";

export function PromptBox({
  title,
  desc,
  placeholder,
  initial,
  confirmLabel,
  onCancel,
  onConfirm,
}) {
  const { t } = useTranslation();
  const [value, setValue] = useState(initial || "");
  const submit = () => onConfirm(value.trim());
  return (
    <ConfirmBox
      width={420}
      title={title}
      actions={(
        <>
          <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onCancel}>
            {t("overlays:prompt.cancel")}
          </button>
          <button
            className="fbtn-primary"
            style={{ height: 34, padding: "0 16px", fontSize: 13 }}
            onClick={submit}
          >
            {confirmLabel || t("overlays:prompt.confirm")}
          </button>
        </>
      )}
    >
      {desc && (
        <div style={{
          fontSize: 12,
          color: "var(--tx3b)",
          marginTop: 7,
          lineHeight: 1.5,
        }}>
          {desc}
        </div>
      )}
      <input
        autoFocus
        value={value}
        placeholder={placeholder}
        onChange={event => setValue(event.target.value)}
        onKeyDown={event => {
          if (event.key === "Enter") submit();
        }}
        style={{
          width: "100%",
          boxSizing: "border-box",
          height: 34,
          marginTop: 12,
          padding: "0 11px",
          background: "var(--surface)",
          border: "1px solid var(--line)",
          borderRadius: 8,
          fontSize: 13,
          color: "var(--tx1)",
        }}
      />
    </ConfirmBox>
  );
}
