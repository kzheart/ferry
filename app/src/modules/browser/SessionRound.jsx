import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { writeClipboardText } from "../../platform/desktop/client.js";
import {
  BookmarkIcon,
  Caret,
  CheckIcon,
  CloseIcon,
  CopyIcon,
  ImageGlyph,
  PencilIcon,
  TrashIcon,
  UndoIcon,
} from "../../shared/ui/icons.jsx";
import Markdown from "../../shared/ui/Markdown.jsx";
import { ACCENT, fmtSize } from "../../shared/ui/toolDisplay.js";
import AssistantReplyEditor from "./AssistantReplyEditor.jsx";

const BIG_OUT = 4096;
const LONG_TEXT = 800;
const LONG_LINES = 12;
const FOLD_MAX_H = 250;

const withoutImagePlaceholders = text => String(text || "")
  .replace(/\s*\[Image #\d+\]/g, "")
  .replace(/\n{3,}/g, "\n\n")
  .trim();

function IconBtn({
  title,
  danger,
  accent,
  onClick,
  style,
  children,
  ...rest
}) {
  return (
    <button
      title={title}
      onClick={onClick}
      {...rest}
      className={
        `ficon-btn${danger ? " danger" : ""}`
        + `${accent ? " accent" : ""}`
      }
      style={style}
    >
      {children}
    </button>
  );
}

function Foldable({ text, fade, children }) {
  const { t: tt } = useTranslation();
  const [open, setOpen] = useState(false);
  const long = text.length > LONG_TEXT
    || (text.match(/\n/g)?.length || 0) > LONG_LINES;
  if (!long) return children;
  return (
    <>
      <div
        style={{
          position: "relative",
          overflow: "hidden",
          maxHeight: open ? undefined : FOLD_MAX_H,
        }}
      >
        {children}
        {!open && (
          <div
            style={{
              position: "absolute",
              left: 0,
              right: 0,
              bottom: 0,
              height: 52,
              pointerEvents: "none",
              background: (
                `linear-gradient(to bottom, transparent, ${fade})`
              ),
            }}
          />
        )}
      </div>
      <button
        type="button"
        onClick={event => {
          event.stopPropagation();
          setOpen(value => !value);
        }}
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 5,
          marginTop: 6,
          padding: 0,
          border: "none",
          background: "transparent",
          color: "var(--tx4)",
          fontFamily: "inherit",
          fontSize: 11,
          fontWeight: 600,
          cursor: "default",
        }}
      >
        <Caret open={open} size={9} />
        {open
          ? tt("browser:round.collapse")
          : tt("browser:round.expand", { n: text.length })}
      </button>
    </>
  );
}

function ToolCard({ tool, tt, open, onToggle }) {
  const big = (tool.size || 0) > BIG_OUT;
  const command = typeof tool.input === "object"
    ? (
      tool.input.command
      || tool.input.file_path
      || tool.input.pattern
      || JSON.stringify(tool.input).slice(0, 80)
    )
    : String(tool.input || "").slice(0, 80);
  const output = tool.output || tt("browser:tool.noOutput");
  return (
    <div
      style={{
        margin: "5px 0",
        border: "1px solid var(--line3)",
        borderRadius: 8,
        overflow: "hidden",
        background: "var(--fill)",
      }}
    >
      <div
        onClick={onToggle}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 9,
          padding: "7px 11px",
          cursor: "default",
        }}
      >
        <Caret open={open} size={10} />
        <span
          className="mono"
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: "var(--tx2b)",
          }}
        >
          {tool.name}
        </span>
        <span
          className="mono"
          style={{
            fontSize: 11,
            color: "var(--tx4)",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
            flex: 1,
          }}
        >
          {command}
        </span>
        {big && (
          <span
            style={{
              fontSize: 10,
              color: "var(--warn-deep)",
              background: "var(--warn-bg)",
              padding: "1px 7px",
              borderRadius: 20,
              flex: "none",
            }}
          >
            {tt("browser:tool.bigOutput", {
              size: fmtSize(tool.size),
            })}
          </span>
        )}
      </div>
      {open && (
        <pre
          className="mono fscroll selectable"
          style={{
            margin: 0,
            padding: "11px 13px",
            fontSize: 11,
            lineHeight: 1.6,
            color: "var(--tx2b)",
            whiteSpace: "pre-wrap",
            maxHeight: 200,
            overflow: "auto",
            background: "var(--surface)",
            borderTop: "1px solid var(--line5)",
          }}
        >
          {output.slice(0, 200000)}
        </pre>
      )}
    </div>
  );
}

export default function SessionRound({
  r,
  editable,
  delOp,
  rewOp,
  onDelete,
  onUndoDelete,
  onRewrite,
  onUpdateRewrite,
  onCancelRewrite,
  migratable,
  replyOp,
  canEditReply,
  replyEditBlocked,
  onStartReply,
  onUpdateReply,
  onCancelReply,
  scopeOn,
  onScope,
  onClearScope,
  onMigrateScope,
  scopeStats,
  onOpenImages,
}) {
  const { t: tt } = useTranslation();
  const [open, setOpen] = useState({});
  const [toolsOpen, setToolsOpen] = useState(false);
  const [rewEditing, setRewEditing] = useState(false);
  const [copied, setCopied] = useState(false);
  const textAreaRef = useRef(null);
  const userText = useMemo(
    () => withoutImagePlaceholders(r.user),
    [r.user],
  );
  const shownUserText = rewOp
    ? withoutImagePlaceholders(rewOp.text)
    : userText;
  const images = r.images || [];
  const fullAiText = r.final || "";
  const aiText = fullAiText.slice(0, 8000);
  const deleted = !!delOp;

  const copyAi = async () => {
    try {
      await writeClipboardText(fullAiText);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {}
  };
  const fitTextArea = element => {
    if (!element) return;
    element.style.height = "auto";
    element.style.height = (
      `${Math.max(element.scrollHeight, 48)}px`
    );
  };
  const startRewrite = () => {
    onRewrite();
    setRewEditing(true);
    setTimeout(() => {
      const element = textAreaRef.current;
      if (element) {
        fitTextArea(element);
        element.focus();
      }
    }, 0);
  };

  return (
    <div
      className="fround"
      data-round={r.n}
      style={{ marginBottom: editable ? 10 : 30 }}
    >
      {editable && (
        <div
          className={deleted || rewOp ? undefined : "fhact"}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 9,
            margin: "10px 0 8px",
          }}
        >
          <span
            style={{
              width: 20,
              height: 20,
              flex: "none",
              borderRadius: "50%",
              border: "1.5px solid var(--line2)",
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              fontSize: 10,
              fontWeight: 700,
              color: "var(--tx4b)",
            }}
          >
            {r.n}
          </span>
          <span
            style={{
              flex: 1,
              height: 1,
              background: "var(--hairline)",
            }}
          />
          <div style={{ display: "flex", gap: 3 }}>
            {deleted ? (
              <IconBtn
                title={tt("browser:round.undoDelete")}
                onClick={onUndoDelete}
              >
                <UndoIcon />
              </IconBtn>
            ) : (
              <IconBtn
                title={tt("browser:round.deleteTurn", {
                  n: r.n,
                })}
                danger
                onClick={onDelete}
              >
                <TrashIcon />
              </IconBtn>
            )}
            {r.locator && !deleted && (
              <IconBtn
                title={tt("browser:round.rewriteUser")}
                onClick={startRewrite}
              >
                <PencilIcon />
              </IconBtn>
            )}
          </div>
        </div>
      )}
      <div className={deleted ? "fdel" : undefined}>
        {images.length > 0 && (
          <div
            style={{
              display: "flex",
              justifyContent: "flex-end",
              margin: "6px 0 4px",
            }}
          >
            <button
              type="button"
              title={tt("browser:round.openImages", {
                n: images.length,
              })}
              onClick={() => onOpenImages(images)}
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
                padding: "4px 10px",
                borderRadius: 20,
                background: "var(--chip)",
                color: "var(--tx3b)",
                fontSize: 11,
                fontWeight: 600,
                border: "1px solid var(--line4)",
                cursor: "pointer",
                boxShadow: "0 1px 0 rgba(255, 255, 255, .08)",
              }}
            >
              <ImageGlyph />{" "}
              {tt("browser:round.viewImages", {
                n: images.length,
              })}
            </button>
          </div>
        )}
        {(shownUserText || (rewOp && rewEditing)) && (
          <div
            style={{
              display: "flex",
              justifyContent: "flex-end",
              margin: "6px 0",
            }}
          >
            {rewOp && rewEditing && !deleted ? (
              <div
                style={{
                  maxWidth: "82%",
                  width: "82%",
                  position: "relative",
                }}
              >
                <textarea
                  ref={element => {
                    textAreaRef.current = element;
                    if (element) fitTextArea(element);
                  }}
                  className="fscroll selectable"
                  value={rewOp.text}
                  onChange={event => {
                    onUpdateRewrite(event.target.value);
                    fitTextArea(event.target);
                  }}
                  onKeyDown={event => {
                    if (event.key === "Escape") {
                      event.preventDefault();
                      onCancelRewrite();
                      setRewEditing(false);
                    }
                    if (
                      (event.metaKey || event.ctrlKey)
                      && event.key === "Enter"
                    ) {
                      event.preventDefault();
                      setRewEditing(false);
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
                    onClick={() => {
                      onCancelRewrite();
                      setRewEditing(false);
                    }}
                  >
                    <CloseIcon />
                  </IconBtn>
                  <IconBtn
                    title={tt("browser:round.confirmRewrite")}
                    accent
                    onClick={() => setRewEditing(false)}
                  >
                    <CheckIcon />
                  </IconBtn>
                </div>
              </div>
            ) : (
              <div
                className="fdel-text selectable"
                onClick={
                  rewOp && !deleted ? startRewrite : undefined
                }
                title={
                  rewOp && !deleted
                    ? tt("browser:round.clickToEdit")
                    : undefined
                }
                style={{
                  maxWidth: "82%",
                  background: "var(--fill4)",
                  color: "var(--tx1b)",
                  padding: "9px 14px",
                  borderRadius: 16,
                  fontSize: 13,
                  lineHeight: 1.65,
                  overflowWrap: "break-word",
                  cursor: (
                    rewOp && !deleted ? "text" : undefined
                  ),
                }}
              >
                <Foldable
                  text={shownUserText}
                  fade="var(--fill4)"
                >
                  <div style={{ whiteSpace: "pre-wrap" }}>
                    {shownUserText.slice(0, 4000)}
                  </div>
                </Foldable>
                {rewOp && !deleted && (
                  <span
                    style={{
                      display: "inline-flex",
                      alignItems: "center",
                      gap: 4,
                      marginLeft: 8,
                      color: ACCENT,
                      fontSize: 10,
                      fontWeight: 600,
                      whiteSpace: "nowrap",
                    }}
                  >
                    <PencilIcon size={10} />{" "}
                    {tt("browser:round.rewritten")}
                  </span>
                )}
              </div>
            )}
          </div>
        )}
        {!replyOp && r.steps.length > 0 && (
          <div style={{ margin: "8px 0" }}>
            <div
              onClick={() => setToolsOpen(value => !value)}
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
                padding: "3px 8px 3px 4px",
                borderRadius: 6,
                cursor: "default",
                color: "var(--tx4)",
                fontSize: 12,
              }}
              className="hov-ghost"
            >
              <Caret open={toolsOpen} size={9} />
              <span>
                {tt("browser:tool.stepCount", {
                  n: r.steps.length,
                })}
              </span>
            </div>
            {toolsOpen && (
              <div
                style={{
                  marginLeft: 18,
                  marginTop: 2,
                  borderLeft: "2px solid var(--line5)",
                  paddingLeft: 13,
                }}
              >
                {r.steps.map((step, index) => (
                  step.kind === "text"
                    ? (
                      <div
                        key={index}
                        className="selectable"
                        style={{
                          margin: "7px 0",
                          fontSize: 12,
                          lineHeight: 1.65,
                          color: "var(--tx3b)",
                          whiteSpace: "pre-wrap",
                          overflowWrap: "break-word",
                        }}
                      >
                        {step.text.slice(0, 4000)}
                      </div>
                    )
                    : (
                      <ToolCard
                        key={index}
                        tool={step.tool}
                        tt={tt}
                        open={open[index] ?? false}
                        onToggle={() => {
                          setOpen(current => ({
                            ...current,
                            [index]: !(current[index] ?? false),
                          }));
                        }}
                      />
                    )
                ))}
              </div>
            )}
          </div>
        )}
        {!replyOp && (aiText || (canEditReply && !deleted)) && (
          <div
            style={{
              margin: aiText ? "10px 0 0" : "6px 0 0",
            }}
          >
            {aiText && (
              <div className="fdel-text">
                <Markdown text={aiText} />
              </div>
            )}
            <div
              className="fhact"
              style={{
                display: "flex",
                gap: 3,
                marginTop: 4,
              }}
            >
              {aiText && (
                <IconBtn
                  title={
                    copied
                      ? tt("browser:round.copiedAi")
                      : tt("browser:round.copyAi")
                  }
                  onClick={copyAi}
                >
                  {copied
                    ? <CheckIcon />
                    : <CopyIcon />}
                </IconBtn>
              )}
              {canEditReply && !deleted && (
                <IconBtn
                  onClick={onStartReply}
                  disabled={replyEditBlocked}
                  title={
                    replyEditBlocked
                      ? tt("browser:replyEditor.blockedHint")
                      : tt("browser:replyEditor.startHint")
                  }
                >
                  <PencilIcon />
                </IconBtn>
              )}
            </div>
          </div>
        )}
        {!deleted && replyOp && (
          <div style={{ marginTop: 7 }}>
            <AssistantReplyEditor
              op={replyOp}
              onChange={onUpdateReply}
              onCancel={onCancelReply}
            />
          </div>
        )}
      </div>
      {migratable && (
        <div style={{ margin: "10px 0 0" }}>
          {!scopeOn ? (
            <div
              className={r.n === 1 ? undefined : "fhact"}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
              }}
            >
              <span
                style={{
                  flex: 1,
                  height: 1,
                  background: "var(--hairline)",
                }}
              />
              <button
                data-guide={r.n === 1 ? "scope" : undefined}
                className="ficon-btn accent"
                onClick={onScope}
                style={{
                  width: "auto",
                  padding: "0 11px",
                  gap: 6,
                  fontSize: 11,
                  fontWeight: 500,
                  border: "1px dashed var(--acc-line2)",
                  borderRadius: 13,
                }}
              >
                <BookmarkIcon />{" "}
                {tt("browser:round.scopeHere")}
              </button>
              <span
                style={{
                  flex: 1,
                  height: 1,
                  background: "var(--hairline)",
                }}
              />
            </div>
          ) : (
            <div
              style={{
                border: "1px solid var(--acc-line2)",
                background: "var(--acc-soft5)",
                borderRadius: 8,
                padding: "11px 13px",
              }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                }}
              >
                <span
                  style={{
                    fontWeight: 600,
                    color: "var(--acc-text)",
                    fontSize: 12,
                  }}
                >
                  {tt("browser:round.scopeOnly", { n: r.n })}
                </span>
                <IconBtn
                  title={tt("browser:round.cancel")}
                  onClick={onClearScope}
                  style={{ marginLeft: "auto" }}
                >
                  <CloseIcon />
                </IconBtn>
              </div>
              <div
                style={{
                  fontSize: 12,
                  color: "var(--tx2b)",
                  marginTop: 5,
                }}
              >
                {scopeStats}
              </div>
              <button
                className="fbtn-primary"
                onClick={onMigrateScope}
                style={{
                  marginTop: 9,
                  height: 28,
                  padding: "0 13px",
                  fontSize: 12,
                }}
              >
                {tt("browser:round.migrateWithScope")}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
