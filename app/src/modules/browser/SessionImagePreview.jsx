import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { engine } from "../../platform/desktop/client.js";
import { ACCENT } from "../../shared/ui/toolDisplay.js";
import {
  Caret,
  CloseIcon,
  ImageGlyph,
  Spinner,
} from "../../shared/ui/icons.jsx";
import { sessionRef } from "./sessionModel.js";

export default function SessionImagePreview({
  images,
  meta,
  onClose,
}) {
  const { t: tt } = useTranslation();
  const [selected, setSelected] = useState(0);
  const [sources, setSources] = useState({});
  const [error, setError] = useState("");
  const [contextMenu, setContextMenu] = useState(null);
  const [copied, setCopied] = useState(false);
  const image = images[selected];
  const source = sources[image.id];

  useEffect(() => {
    const closeOnEscape = event => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  useEffect(() => {
    if (source) return;
    let cancelled = false;
    setError("");
    engine("session_asset", {
      tool: meta.tool,
      ref: sessionRef(meta),
      asset_id: image.id,
    })
      .then(asset => {
        if (!cancelled) {
          setSources(current => ({
            ...current,
            [image.id]: (
              `data:${asset.mime_type};base64,${asset.data}`
            ),
          }));
        }
      })
      .catch(() => {
        if (!cancelled) {
          setError(tt("browser:round.imageLoadFailed"));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [image.id, meta, source, tt]);

  const choose = index => {
    setSelected(index);
    setError("");
  };
  const previous = () => {
    choose((selected + images.length - 1) % images.length);
  };
  const next = () => {
    choose((selected + 1) % images.length);
  };
  const copyImage = async () => {
    try {
      if (
        !navigator.clipboard?.write
        || typeof ClipboardItem === "undefined"
      ) {
        throw new Error();
      }
      const blob = await (await fetch(source)).blob();
      await navigator.clipboard.write([
        new ClipboardItem({ [blob.type]: blob }),
      ]);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      setError(tt("browser:round.imageCopyFailed"));
    }
    setContextMenu(null);
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={tt("browser:round.imagePreview")}
      onMouseDown={event => {
        setContextMenu(null);
        if (event.target === event.currentTarget) onClose();
      }}
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 20,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: 22,
        background: "rgba(7, 9, 13, .8)",
        backdropFilter: "blur(9px)",
      }}
    >
      <div
        style={{
          width: "min(940px, 100%)",
          maxHeight: "min(760px, 100%)",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          border: "1px solid var(--line3)",
          borderRadius: 14,
          background: "var(--bg)",
          boxShadow: "0 28px 90px rgba(0, 0, 0, .45)",
        }}
      >
        <div
          style={{
            height: 48,
            display: "flex",
            alignItems: "center",
            padding: "0 12px 0 16px",
            borderBottom: "1px solid var(--line5)",
            gap: 10,
          }}
        >
          <ImageGlyph size={14} />
          <span
            style={{
              fontSize: 12,
              fontWeight: 650,
              color: "var(--tx2)",
              flex: 1,
            }}
          >
            {tt("browser:round.imagePosition", {
              current: selected + 1,
              total: images.length,
            })}
          </span>
          {copied && (
            <span
              style={{
                fontSize: 11,
                color: "var(--ok)",
                fontWeight: 600,
              }}
            >
              {tt("browser:round.imageCopied")}
            </span>
          )}
          <button
            type="button"
            className="ficon-btn"
            title={tt("browser:round.closeImagePreview")}
            onClick={onClose}
          >
            <CloseIcon />
          </button>
        </div>
        <div
          style={{
            minHeight: 220,
            flex: 1,
            position: "relative",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            padding: 16,
            overflow: "auto",
            background: "var(--surface)",
          }}
        >
          {!source && !error && <Spinner size={20} />}
          {error && (
            <span
              style={{
                color: "var(--err-text)",
                fontSize: 12,
              }}
            >
              {error}
            </span>
          )}
          {source && (
            <img
              src={source}
              alt={
                image.filename
                || tt("browser:round.imageAlt", {
                  n: selected + 1,
                })
              }
              onContextMenu={event => {
                event.preventDefault();
                setContextMenu({
                  x: Math.min(
                    event.clientX,
                    window.innerWidth - 178,
                  ),
                  y: Math.min(
                    event.clientY,
                    window.innerHeight - 54,
                  ),
                });
              }}
              style={{
                maxWidth: "100%",
                maxHeight: "calc(min(760px, 100vh) - 148px)",
                display: "block",
                objectFit: "contain",
                borderRadius: 6,
                boxShadow: "0 6px 24px rgba(0, 0, 0, .2)",
              }}
            />
          )}
          {images.length > 1 && (
            <>
              <button
                type="button"
                title={tt("browser:round.previousImage")}
                onClick={previous}
                style={{
                  position: "absolute",
                  left: 12,
                  top: "50%",
                  transform: (
                    "translateY(-50%) rotate(180deg)"
                  ),
                  width: 34,
                  height: 34,
                  display: "inline-flex",
                  alignItems: "center",
                  justifyContent: "center",
                  border: "1px solid var(--line3)",
                  borderRadius: "50%",
                  background: "var(--bg)",
                  color: "var(--tx2)",
                  cursor: "default",
                }}
              >
                <Caret open={false} size={15} />
              </button>
              <button
                type="button"
                title={tt("browser:round.nextImage")}
                onClick={next}
                style={{
                  position: "absolute",
                  right: 12,
                  top: "50%",
                  transform: "translateY(-50%)",
                  width: 34,
                  height: 34,
                  display: "inline-flex",
                  alignItems: "center",
                  justifyContent: "center",
                  border: "1px solid var(--line3)",
                  borderRadius: "50%",
                  background: "var(--bg)",
                  color: "var(--tx2)",
                  cursor: "default",
                }}
              >
                <Caret open={false} size={15} />
              </button>
            </>
          )}
        </div>
        {images.length > 1 && (
          <div
            style={{
              display: "flex",
              justifyContent: "center",
              gap: 6,
              padding: 10,
              borderTop: "1px solid var(--line5)",
              overflowX: "auto",
            }}
          >
            {images.map((item, index) => (
              <button
                key={item.id}
                type="button"
                onClick={() => choose(index)}
                title={tt("browser:round.imagePosition", {
                  current: index + 1,
                  total: images.length,
                })}
                style={{
                  width: index === selected ? 18 : 6,
                  height: 6,
                  flex: "none",
                  padding: 0,
                  border: "none",
                  borderRadius: 8,
                  background: (
                    index === selected
                      ? ACCENT
                      : "var(--line2)"
                  ),
                  cursor: "default",
                  transition: "width .16s ease",
                }}
              />
            ))}
          </div>
        )}
      </div>
      {contextMenu && (
        <div
          role="menu"
          onMouseDown={event => event.stopPropagation()}
          style={{
            position: "fixed",
            zIndex: 21,
            left: contextMenu.x,
            top: contextMenu.y,
            minWidth: 166,
            padding: 5,
            border: "1px solid var(--line2)",
            borderRadius: 10,
            background: "var(--bg)",
            boxShadow: "0 14px 38px rgba(0, 0, 0, .32)",
          }}
        >
          <button
            role="menuitem"
            type="button"
            onClick={copyImage}
            style={{
              width: "100%",
              height: 32,
              padding: "0 10px",
              display: "flex",
              alignItems: "center",
              border: "none",
              borderRadius: 6,
              background: "transparent",
              color: "var(--tx1)",
              fontFamily: "inherit",
              fontSize: 12,
              textAlign: "left",
              cursor: "pointer",
            }}
          >
            {tt("browser:round.copyImage")}
          </button>
        </div>
      )}
    </div>
  );
}
