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
  delete: "kindDelete",
};

const formatBytes = value => {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  const amount = value / (1024 ** index);
  return `${amount >= 10 || index === 0 ? Math.round(amount) : amount.toFixed(1)} ${units[index]}`;
};

const formatUpdated = value => {
  if (value == null || value === "") return "—";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? String(value) : parsed.toLocaleString();
};

const CAUSE_KEYS = {
  pinned: "causePinned",
  archived: "causeArchived",
  tagged: "causeTagged",
};

function DeletePreview({ preview, t }) {
  const [expanded, setExpanded] = useState(false);
  const totals = preview.totals || {};
  const sessions = Array.isArray(preview.sessions) ? preview.sessions : [];
  const excluded = Array.isArray(preview.excluded) ? preview.excluded : [];
  return (
    <div style={{
      display: "flex",
      flexDirection: "column",
      gap: 9,
      padding: "10px 11px",
      borderRadius: 8,
      background: "var(--inset)",
      border: "1px solid var(--line4)",
    }}>
      <div style={{ fontSize: 11, fontWeight: 600, color: "var(--tx2)" }}>
        {t("askferry:deletion.previewTitle")}
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 7 }}>
        <div>
          <div style={{ color: "var(--tx5)", fontSize: 10.5 }}>
            {t("askferry:deletion.totalCount")}
          </div>
          <div style={{ color: "var(--tx1)", fontSize: 13, fontWeight: 600 }}>
            {totals.count ?? 0}
          </div>
        </div>
        <div>
          <div style={{ color: "var(--tx5)", fontSize: 10.5 }}>
            {t("askferry:deletion.totalSize")}
          </div>
          <div className="mono" style={{ color: "var(--tx1)", fontSize: 12.5 }}>
            {formatBytes(totals.size_bytes)}
          </div>
        </div>
        <div>
          <div style={{ color: "var(--tx5)", fontSize: 10.5 }}>
            {t("askferry:deletion.tool")}
          </div>
          <div style={{ color: "var(--tx1)", fontSize: 12.5, fontWeight: 600 }}>
            {preview.tool || "—"}
          </div>
        </div>
      </div>

      <div style={{ color: "var(--err-text)", fontSize: 11, fontWeight: 600 }}>
        {t("askferry:deletion.permanent")}
      </div>

      {sessions.length > 0 && (
        <div>
          <button className="fbtn" type="button" onClick={() => setExpanded(value => !value)}>
            {t(expanded ? "askferry:deletion.hideSessions" : "askferry:deletion.showSessions", {
              n: sessions.length,
            })}
          </button>
          {expanded && (
            <div style={{ display: "flex", flexDirection: "column", gap: 5, marginTop: 7 }}>
              {sessions.map(session => (
                <div key={`${session.tool}:${session.ref}`} style={{
                  padding: "7px 8px",
                  borderRadius: 6,
                  background: "var(--surface)",
                  border: "1px solid var(--line5)",
                  fontSize: 11,
                }}>
                  <div style={{ display: "flex", gap: 8, alignItems: "baseline" }}>
                    <span className="selectable" style={{ color: "var(--tx1)", fontWeight: 600 }}>
                      {session.title || session.ref || "—"}
                    </span>
                    <span style={{ flex: 1 }} />
                    <span style={{ color: "var(--tx5)" }}>
                      {formatUpdated(session.updated)}
                    </span>
                  </div>
                  <div className="selectable" style={{ color: "var(--tx4)", marginTop: 3 }}>
                    {session.project || "—"}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {excluded.length > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <div style={{ color: "var(--warn-text)", fontSize: 10.5 }}>
            {t("askferry:deletion.excluded", { n: excluded.length })}
          </div>
          {excluded.map(entry => (
            <div key={`${entry.tool}:${entry.ref}`} style={{ display: "flex", gap: 8,
              color: "var(--tx4)", fontSize: 11 }}>
              <span className="selectable">{entry.title || entry.ref}</span>
              <span style={{ flex: 1 }} />
              <span>{t(`askferry:deletion.${CAUSE_KEYS[entry.cause] || "causeGeneric"}`)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function ApprovalCard({
  item,
  onApprove,
  onDismiss,
  onNavigate,
}) {
  const { t } = useTranslation();
  const operation = item.operation || {};
  const deletion = operation.kind === "delete";
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
      {deletion && <DeletePreview preview={operation.preview || {}} t={t} />}
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
