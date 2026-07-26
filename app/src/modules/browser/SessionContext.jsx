import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Caret } from "../../shared/ui/icons.jsx";
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

const pad2 = n => String(n).padStart(2, "0");

function timeOf(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

function statusOf(compaction, tt) {
  const summary = compaction.summary || {};
  return compaction.state === "in_progress"
    ? tt("browser:context.inProgress")
    : summary.status === "available"
      ? tt("browser:context.summaryAvailable")
      : summary.status === "protected"
        ? tt("browser:context.summaryProtected")
        : tt("browser:context.summaryMissing");
}

// 压缩详情:默认收起,只在展开分割线后出现,所以用最轻的容器,不再自带边框强调
function CompactionDetail({ compaction, title }) {
  const { t: tt } = useTranslation();
  const [open, setOpen] = useState(false);
  const summary = compaction.summary || {};
  const readable = summary.status === "available" && !!summary.text;
  const metrics = compaction.metrics || {};
  const facts = [
    statusOf(compaction, tt),
    compaction.tail?.status === "located"
      && Number.isInteger(compaction.tail.start_message_index)
      ? tt("browser:context.tailStartsAt", {
        n: compaction.tail.start_message_index,
      })
      : null,
    Number.isInteger(metrics.pre_tokens) && Number.isInteger(metrics.post_tokens)
      ? tt("browser:context.tokenChange", {
        before: metrics.pre_tokens.toLocaleString(),
        after: metrics.post_tokens.toLocaleString(),
      })
      : null,
  ].filter(Boolean);

  return (
    <div style={{ fontSize: 11.5, color: "var(--tx4)", lineHeight: 1.6 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span>
          {title ? <span style={{ color: "var(--tx3b)" }}>{title} · </span> : null}
          {facts.join(" · ")}
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
      <div>
        {tt("browser:context.resumeHint")}
        {summary.status === "protected" && (
          <> · {tt("browser:context.protectedHint")}</>
        )}
      </div>
      {open && readable && (
        <div
          style={{
            marginTop: 8,
            paddingTop: 8,
            borderTop: "1px solid var(--line5)",
            color: "var(--tx2)",
            fontSize: 12,
          }}
        >
          <Markdown text={summary.text} />
        </div>
      )}
    </div>
  );
}

// 上下文压缩边界:正文流里只留一条细分割线,详情按需展开,避免整块卡片抢视线
export function CompactionBoundary({ compaction, compactions }) {
  const { t: tt } = useTranslation();
  const [open, setOpen] = useState(false);
  const list = compactions?.length ? compactions : [compaction].filter(Boolean);
  if (!list.length) return null;

  const single = list.length === 1 ? list[0] : null;
  const trigger = !single
    ? null
    : single.trigger === "automatic"
      ? tt("browser:context.automatic")
      : single.trigger === "manual"
        ? tt("browser:context.manual")
        : null;
  const label = single
    ? tt("browser:context.boundaryChip")
    : tt("browser:context.boundaryChipCount", { n: list.length });
  const line = { flex: 1, height: 1, background: "var(--line5)" };

  return (
    <div style={{ margin: "14px 0 18px" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <span style={line} />
        <button
          type="button"
          onClick={() => setOpen(value => !value)}
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: 5,
            border: 0,
            padding: 0,
            background: "transparent",
            color: "var(--tx4)",
            fontSize: 11,
            cursor: "pointer",
          }}
        >
          {label}
          {trigger ? ` · ${trigger}` : null}
          <Caret size={8} open={open} />
        </button>
        <span style={line} />
      </div>
      {open && (
        <div
          style={{
            marginTop: 9,
            padding: "9px 12px",
            borderRadius: 8,
            background: "var(--surface)",
            border: "1px solid var(--line5)",
            display: "flex",
            flexDirection: "column",
            gap: 10,
          }}
        >
          {list.map((item, index) => {
            const time = timeOf(item.created_at);
            const title = single
              ? null
              : tt("browser:context.groupItemTitle", { i: index + 1 })
                + (time ? ` · ${time}` : "");
            return (
              <CompactionDetail
                key={item.id}
                compaction={item}
                title={title}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}
