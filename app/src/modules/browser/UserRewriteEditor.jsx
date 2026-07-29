// 用户消息改写编辑器:自适应高度 textarea + 取消/确认;Esc 取消、⌘⏎ 确认。
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { CheckIcon, CloseIcon } from "../../shared/ui/icons.jsx";
import { ACCENT } from "../../shared/ui/toolDisplay.js";

const fit = element => {
  if (!element) return;
  element.style.height = "auto";
  element.style.height = `${Math.max(element.scrollHeight, 48)}px`;
};

function IconBtn({ title, accent, onClick, children }) {
  return (
    <button
      title={title}
      onClick={onClick}
      className={`ficon-btn${accent ? " accent" : ""}`}
    >
      {children}
    </button>
  );
}

export default function UserRewriteEditor({
  text,
  onChange,
  onCancel,
  onDone,
}) {
  const { t: tt } = useTranslation();
  const textAreaRef = useRef(null);
  useEffect(() => {
    const element = textAreaRef.current;
    if (element) {
      fit(element);
      element.focus();
    }
  }, []);

  return (
    <div style={{ maxWidth: "82%", width: "82%", position: "relative" }}>
      <textarea
        ref={element => {
          textAreaRef.current = element;
          if (element) fit(element);
        }}
        className="fscroll selectable"
        value={text}
        onChange={event => {
          onChange(event.target.value);
          fit(event.target);
        }}
        onKeyDown={event => {
          if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
          }
          if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
            event.preventDefault();
            onDone();
          }
        }}
        style={{
          width: "100%",
          display: "block",
          resize: "none",
          overflow: "hidden",
          boxSizing: "border-box",
          background: "var(--fill4)",
          color: "var(--tx1b)",
          border: `1.5px solid ${ACCENT}`,
          padding: "9px 14px",
          borderRadius: 16,
          fontSize: 13,
          lineHeight: 1.65,
          userSelect: "text",
          fontFamily: "inherit",
          whiteSpace: "pre-wrap",
          overflowWrap: "break-word",
        }}
      />
      <div
        style={{
          display: "flex",
          justifyContent: "flex-end",
          gap: 3,
          marginTop: 6,
        }}
      >
        <IconBtn
          title={tt("browser:round.cancelRewrite")}
          onClick={onCancel}
        >
          <CloseIcon />
        </IconBtn>
        <IconBtn
          title={tt("browser:round.confirmRewrite")}
          accent
          onClick={onDone}
        >
          <CheckIcon />
        </IconBtn>
      </div>
    </div>
  );
}
