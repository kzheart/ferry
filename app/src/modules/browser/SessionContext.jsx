import { useState } from "react";
import { useTranslation } from "react-i18next";

import Markdown from "../../shared/ui/Markdown.jsx";

export function ContextStatusChip({ context }) {
  const { t: tt } = useTranslation();
  if (!context || context.state === "full") return null;
  const summaryKey = context.summary_status === "available"
    ? "summaryAvailable"
    : context.summary_status === "protected"
      ? "summaryProtected"
      : "summaryMissing";
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 5,
        padding: "2px 7px",
        borderRadius: 6,
        color: "var(--warn-text)",
        background: "var(--warn-bg)",
        border: "1px solid var(--warn-line)",
      }}
    >
      {context.state === "in_progress"
        ? tt("browser:context.inProgress")
        : tt("browser:context.compactedCount", {
          n: context.compaction_count,
        })}
      {context.state !== "in_progress" && (
        <> · {tt(`browser:context.${summaryKey}`)}</>
      )}
    </span>
  );
}

export function CompactionBoundary({ compaction }) {
  const { t: tt } = useTranslation();
  const [open, setOpen] = useState(false);
  const summary = compaction.summary || {};
  const readable = (
    summary.status === "available" && !!summary.text
  );
  const trigger = compaction.trigger === "automatic"
    ? tt("browser:context.automatic")
    : compaction.trigger === "manual"
      ? tt("browser:context.manual")
      : tt("browser:context.triggerUnknown");
  const status = compaction.state === "in_progress"
    ? tt("browser:context.inProgress")
    : summary.status === "available"
      ? tt("browser:context.summaryAvailable")
      : summary.status === "protected"
        ? tt("browser:context.summaryProtected")
        : tt("browser:context.summaryMissing");
  const metrics = compaction.metrics || {};

  return (
    <div style={{ margin: "18px 0 24px" }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 10,
        }}
      >
        <span
          style={{
            flex: 1,
            height: 1,
            background: "var(--warn-line)",
          }}
        />
        <span
          style={{
            padding: "3px 9px",
            borderRadius: 999,
            border: "1px solid var(--warn-line)",
            color: "var(--warn-text)",
            background: "var(--warn-bg)",
            fontSize: 11,
            fontWeight: 650,
          }}
        >
          {tt("browser:context.boundaryTitle")} · {trigger}
        </span>
        <span
          style={{
            flex: 1,
            height: 1,
            background: "var(--warn-line)",
          }}
        />
      </div>
      <div
        style={{
          marginTop: 9,
          padding: "11px 13px",
          borderRadius: 9,
          border: "1px solid var(--warn-line)",
          background: "var(--surface)",
          color: "var(--tx3b)",
          fontSize: 12,
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
              color: "var(--warn-text)",
              fontWeight: 650,
            }}
          >
            {status}
          </span>
          <span style={{ flex: 1 }} />
          {readable && (
            <button
              type="button"
              onClick={() => setOpen(value => !value)}
              style={{
                border: 0,
                padding: 0,
                background: "transparent",
                color: "var(--tx3b)",
                font: "inherit",
                cursor: "pointer",
              }}
            >
              {open
                ? tt("browser:context.hideSummary")
                : tt("browser:context.showSummary")}
            </button>
          )}
        </div>
        <div
          style={{
            marginTop: 5,
            color: "var(--tx4)",
            lineHeight: 1.5,
          }}
        >
          {tt("browser:context.resumeHint")}
          {compaction.tail?.status === "located"
            && Number.isInteger(
              compaction.tail.start_message_index,
            )
            && (
              <> · {tt("browser:context.tailStartsAt", {
                n: compaction.tail.start_message_index,
              })}</>
            )}
          {Number.isInteger(metrics.pre_tokens)
            && Number.isInteger(metrics.post_tokens)
            && (
              <> · {tt("browser:context.tokenChange", {
                before: metrics.pre_tokens.toLocaleString(),
                after: metrics.post_tokens.toLocaleString(),
              })}</>
            )}
        </div>
        {summary.status === "protected" && (
          <div
            style={{
              marginTop: 6,
              color: "var(--tx4)",
            }}
          >
            {tt("browser:context.protectedHint")}
          </div>
        )}
        {open && readable && (
          <div
            style={{
              marginTop: 11,
              paddingTop: 11,
              borderTop: "1px solid var(--line5)",
              color: "var(--tx2)",
            }}
          >
            <Markdown text={summary.text} />
          </div>
        )}
      </div>
    </div>
  );
}
