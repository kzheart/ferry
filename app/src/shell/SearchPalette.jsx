import { useEffect, useRef, useState } from "react";

import {
  CloseIcon,
  SearchIcon,
  ToolIcon,
} from "../shared/ui/icons.jsx";

const sectionLabelStyle = {
  fontSize: 11,
  fontWeight: 600,
  color: "var(--tx4)",
  padding: "6px 10px 4px",
};

export function SearchPalette({
  placeholder,
  query,
  onQuery,
  results,
  recentLabel,
  emptyLabel,
  notice,
  onClose,
}) {
  const [selectedIndex, setSelectedIndex] = useState(0);
  const rowsRef = useRef([]);
  // 鼠标只有真的动过才有权改选中:方向键会滚动列表,新行滑到静止的光标底下时
  // 浏览器照样派发 mouseenter,不设这道闸,高亮会被"划过"的行抢走。
  const mouseLive = useRef(false);
  useEffect(() => setSelectedIndex(0), [query]);
  // 键盘选中要跟着滚:结果超过一屏时,高亮不能停在视口外
  useEffect(() => {
    rowsRef.current[selectedIndex]?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex, results]);
  useEffect(() => {
    const onKey = event => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      } else if (event.key === "ArrowDown") {
        event.preventDefault();
        mouseLive.current = false;
        setSelectedIndex(index => Math.min(index + 1, results.length - 1));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        mouseLive.current = false;
        setSelectedIndex(index => Math.max(index - 1, 0));
      } else if (event.key === "Enter") {
        event.preventDefault();
        results[selectedIndex]?.onClick?.();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [results, selectedIndex, onClose]);

  return (
    <div
      onClick={onClose}
      style={{
        position: "absolute",
        inset: 0,
        zIndex: 70,
        background: "var(--dim)",
        display: "flex",
        justifyContent: "center",
        alignItems: "flex-start",
        paddingTop: "9vh",
      }}
    >
      <div
        onClick={event => event.stopPropagation()}
        className="fsheet"
        style={{
          width: "min(680px, 78vw)",
          maxHeight: "76vh",
          display: "flex",
          flexDirection: "column",
          background: "var(--bg)",
          borderRadius: 14,
          boxShadow: "var(--shadow-sheet)",
          overflow: "hidden",
        }}
      >
        <div style={{
          display: "flex",
          alignItems: "center",
          gap: 11,
          padding: "0 14px",
          height: 52,
          borderBottom: "1px solid var(--line5)",
          flex: "none",
        }}>
          <span style={{ color: "var(--tx4)", display: "inline-flex" }}>
            <SearchIcon />
          </span>
          <input
            autoFocus
            value={query}
            onChange={onQuery}
            placeholder={placeholder}
            style={{
              flex: 1,
              border: "none",
              background: "transparent",
              fontSize: 15,
              color: "var(--tx1)",
              outline: "none",
            }}
          />
          <button className="ftool-btn" onClick={onClose}>
            <CloseIcon size={13} />
          </button>
        </div>
        <div className="fscroll" onMouseMove={() => { mouseLive.current = true; }}
          style={{ overflowY: "auto", padding: 8, minHeight: 0 }}>
          {recentLabel && <div style={sectionLabelStyle}>{recentLabel}</div>}
          {notice && (
            <div style={{
              margin: "4px 2px 6px",
              padding: "8px 10px",
              borderRadius: 8,
              background: "var(--acc-soft2)",
              color: "var(--tx4)",
              fontSize: 12,
            }}>
              {notice}
            </div>
          )}
          {results.length === 0 && !notice ? (
            <div style={{
              padding: "26px 12px",
              textAlign: "center",
              color: "var(--tx5)",
              fontSize: 13,
            }}>
              {emptyLabel}
            </div>
          ) : results.map((result, index) => (
            <div key={result.id}>
              {result.section && result.section !== results[index - 1]?.section
                && <div style={sectionLabelStyle}>{result.section}</div>}
            <div
              ref={node => { rowsRef.current[index] = node; }}
              onMouseEnter={() => { if (mouseLive.current) setSelectedIndex(index); }}
              onClick={() => {
                result.onClick?.();
                onClose();
              }}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 10,
                padding: "0 10px",
                height: 42,
                borderRadius: 8,
                cursor: "default",
                background: index === selectedIndex
                  ? "var(--acc-soft2)"
                  : "transparent",
              }}
            >
              {result.tool && <ToolIcon tool={result.tool} size={20} />}
              <span style={{
                flex: 1,
                minWidth: 0,
                fontSize: 13,
                color: "var(--tx1)",
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
              }}>
                {result.title}
              </span>
              {result.badge && (
                <span style={{
                  flex: "none",
                  fontSize: 10,
                  lineHeight: "16px",
                  padding: "0 6px",
                  borderRadius: 4,
                  color: "var(--tx4)",
                  border: "1px solid var(--line5)",
                }}>
                  {result.badge}
                </span>
              )}
              {result.meta && (
                <span className="mono" style={{
                  fontSize: 11,
                  color: "var(--tx5)",
                  flex: "none",
                  maxWidth: "42%",
                  whiteSpace: "nowrap",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                }}>
                  {result.meta}
                </span>
              )}
            </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
