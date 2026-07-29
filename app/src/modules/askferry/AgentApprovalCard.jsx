import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { entitiesFromToolResult } from "./ferryEntities.js";
import EntityCards from "./EntityCards.jsx";

function Countdown({ until }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(
    0,
    Math.floor(((until || 0) - now) / 1000),
  );
  return (
    <span className="mono">
      {Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}
    </span>
  );
}

const KIND_KEYS = {
  migration: "kindMigration",
  edit: "kindEdit",
  metadata: "kindMetadata",
};

export function ApprovalCard({
  item,
  onApprove,
  onDismiss,
  onNavigate,
}) {
  const { t } = useTranslation();
  const operation = item.operation || {};
  const applied = item.status === "applied";
  const failed = item.status === "failed";
  const expired = item.status === "pending"
    && operation.expires_at
    && operation.expires_at < Date.now();
  const dot = applied
    ? "var(--ok)"
    : failed
      ? "var(--err)"
      : "var(--warn)";
  const title = applied
    ? t("askferry:approval.applied")
    : failed
      ? t("askferry:approval.failed")
      : item.status === "applying"
        ? t("askferry:approval.applying")
        : item.status === "dismissed"
          ? t("askferry:approval.dismissed")
          : t(`askferry:approval.${KIND_KEYS[operation.kind] || "kindGeneric"}`);
  const entities = operation.kind === "migration" || operation.kind === "edit"
    ? entitiesFromToolResult(
      operation.kind === "migration" ? "migrate" : "session_edit",
      { details: { ...operation, result: item.result } },
    )
    : [];
  return (
    <div className="fcard" style={{
      padding: "12px 14px",
      display: "flex",
      flexDirection: "column",
      gap: 8,
      maxWidth: 560,
    }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={{
          width: 7,
          height: 7,
          borderRadius: "50%",
          background: dot,
          flex: "none",
        }} />
        <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx1)" }}>
          {title}
        </span>
        <span style={{ flex: 1 }} />
        {item.status === "pending" && operation.expires_at && !expired && (
          <span style={{ fontSize: 11, color: "var(--tx5)" }}>
            {t("askferry:approval.expires")}{" "}
            <Countdown until={operation.expires_at} />
          </span>
        )}
      </div>
      {operation.summary && (
        <div className="selectable" style={{
          fontSize: 12.5,
          color: "var(--tx2)",
          lineHeight: 1.55,
        }}>
          {operation.summary}
        </div>
      )}
      <EntityCards entities={entities} onNavigate={onNavigate} />
      <div style={{
        display: "flex",
        gap: 12,
        flexWrap: "wrap",
        fontSize: 11,
        color: "var(--tx4)",
      }}>
        {Array.isArray(operation.affected_refs) && (
          <span>
            {t("askferry:approval.affected", {
              n: operation.affected_refs.length,
            })}
          </span>
        )}
        {operation.risk && (
          <span>
            {t("askferry:approval.risk", { risk: operation.risk })}
          </span>
        )}
        {expired && (
          <span style={{ color: "var(--err-text)" }}>
            {t("askferry:approval.expired")}
          </span>
        )}
      </div>
      {failed && item.error && (
        <div className="mono selectable" style={{
          fontSize: 11,
          color: "var(--err-text)",
        }}>
          {item.error}
        </div>
      )}
      {applied && item.result?.saved_as && (
        <div className="mono selectable" style={{
          fontSize: 11,
          color: "var(--tx4)",
        }}>
          {item.result.saved_as}
        </div>
      )}
      {item.status === "pending" && !expired && (
        <div style={{
          display: "flex",
          gap: 8,
          justifyContent: "flex-end",
          marginTop: 2,
        }}>
          <button className="fbtn" onClick={onDismiss}>
            {t("askferry:approval.reject")}
          </button>
          <button className="fbtn fbtn-primary" onClick={onApprove}>
            {t("askferry:approval.approve")}
          </button>
        </div>
      )}
    </div>
  );
}

