// 破坏性操作确认框:说明文案 + 彩色圆点要点列表 + 红色确认按钮。
// 破坏性确认共用同一骨架,只有文案与要点不同。
import { useTranslation } from "react-i18next";

import { ConfirmBox } from "./ConfirmBox.jsx";

// bullets: [color, text][]
export function DangerConfirm({
  width = 430,
  title,
  desc,
  bullets,
  confirmLabel,
  onCancel,
  onConfirm,
}) {
  const { t } = useTranslation();
  return (
    <ConfirmBox
      width={width}
      title={title}
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
            {confirmLabel}
          </button>
        </>
      )}
    >
      {desc && (
        <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 7, lineHeight: 1.5 }}>
          {desc}
        </div>
      )}
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
