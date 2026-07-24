import { useEffect, useState } from "react";

import {
  CloseIcon,
  SearchIcon,
  ToolIcon,
} from "../shared/ui/icons.jsx";

export function SearchPalette({
  placeholder,
  query,
  onQuery,
  results,
  recentLabel,
  emptyLabel,
  onClose,
}) {
  const [selectedIndex, setSelectedIndex] = useState(0);
  useEffect(() => setSelectedIndex(0), [query]);
  useEffect(() => {
    const onKey = event => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      } else if (event.key === "ArrowDown") {
        event.preventDefault();
        setSelectedIndex(index => Math.min(index + 1, results.length - 1));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
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
        <div className="fscroll" style={{ overflowY: "auto", padding: 8, minHeight: 0 }}>
          {recentLabel && (
            <div style={{
              fontSize: 11,
              fontWeight: 600,
              color: "var(--tx4)",
              padding: "6px 10px 4px",
            }}>
              {recentLabel}
            </div>
          )}
          {results.length === 0 ? (
            <div style={{
              padding: "26px 12px",
              textAlign: "center",
              color: "var(--tx5)",
              fontSize: 13,
            }}>
              {emptyLabel}
            </div>
          ) : results.map((result, index) => (
            <div
              key={result.id}
              onMouseEnter={() => setSelectedIndex(index)}
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
          ))}
        </div>
      </div>
    </div>
  );
}
