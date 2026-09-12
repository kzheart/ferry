import { useTranslation } from "react-i18next";
import "./AgentRunStatus.css";

export function classifyRunFailure(message) {
  const text = typeof message === "string" ? message : "";
  if (/\b(?:invalid_api_key|authentication_error|incorrect api key|invalid api key|unauthorized)\b/i.test(text)) {
    return "auth";
  }
  if (/\b(?:rate_limit_exceeded|rate limit(?:ed| exceeded)?|too many requests)\b/i.test(text)) {
    return "rateLimit";
  }
  if (/\b(?:connection error|connection refused|ECONNREFUSED|ENOTFOUND|EAI_AGAIN|fetch failed)\b/i.test(text)) {
    return "connection";
  }
  return "generic";
}

export function AgentRunStatus({ item }) {
  const { t } = useTranslation();
  if (item.type === "run.failed") {
    const category = classifyRunFailure(item.message);
    return (
      <section className="agent-run-feedback selectable" aria-label={t("askferry:chat.failure.title")}>
        <div className="agent-run-feedback-title">
          <span className="agent-run-feedback-icon" aria-hidden="true">!</span>
          {t(`askferry:chat.failure.${category}Title`)}
        </div>
        <p>{t(`askferry:chat.failure.${category}Hint`)}</p>
        {item.message && (
          <details className="agent-run-feedback-details">
            <summary>{t("askferry:chat.failure.details")}</summary>
            <pre className="selectable">{item.message}</pre>
          </details>
        )}
      </section>
    );
  }
  const key = {
    "context.compacted": "contextCompacted",
    "run.cancelled": "runCancelled",
    "run.interrupted": "runInterrupted",
  }[item.type];
  return (
    <div className="agent-run-note selectable">
      <span className="agent-run-note-mark" aria-hidden="true" />
      {key ? t(`askferry:chat.${key}`) : item.type}
    </div>
  );
}
