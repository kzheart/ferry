import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { entitiesFromToolResult } from "./ferryEntities.js";
import EntityCards from "./EntityCards.jsx";
import "./agentApproval.css";

const KIND_KEYS = {
  migration: "kindMigration",
  edit: "kindEdit",
  metadata: "kindMetadata",
};

export function ApprovalCard({ item, onApprove, onDismiss, onNavigate }) {
  const { t } = useTranslation();
  const operation = item.operation || {};
  const [now, setNow] = useState(Date.now());
  const pending = item.status === "pending";
  const until = operation.expires_at;
  useEffect(() => {
    if (!pending || !until) return;
    let timer;
    const tick = () => {
      const current = Date.now();
      setNow(current);
      if (current < until) timer = setTimeout(tick, Math.min(1000, until - current));
    };
    tick();
    return () => clearTimeout(timer);
  }, [pending, until]);
  const expired = pending && !!until && until <= now;
  const applied = item.status === "applied";
  const failed = item.status === "failed";
  const active = pending && !expired;
  const seconds = Math.max(0, Math.ceil(((until || 0) - now) / 1000));
  const titleKey = applied ? "applied"
    : failed ? "failed"
      : item.status === "applying" ? "applying"
        : item.status === "dismissed" ? "dismissed"
          : expired ? "expired"
            : KIND_KEYS[operation.kind] || "kindGeneric";
  const entities = operation.kind === "migration" || operation.kind === "edit"
    ? entitiesFromToolResult(
      operation.kind === "migration" ? "migrate" : "session_edit",
      { details: { ...operation, result: item.result } },
    ) : [];
  const approve = () => {
    // 事件处理也检查截止时间，避免计时器尚未执行时批准刚过期的提议。
    if (!pending || (until && until <= Date.now())) {
      setNow(Date.now());
      return;
    }
    onApprove();
  };
  return (
    <section className={`agent-approval ${active ? "is-pending" : "is-complete"} ${failed ? "is-failed" : ""}`}>
      <div className="agent-approval-heading">
        <span className="agent-approval-status">{t(`askferry:approval.${titleKey}`)}</span>
        {active && until && (
          <span className="agent-approval-countdown">
            {t("askferry:approval.expires")}{" "}
            <span className="mono">{Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}</span>
          </span>
        )}
      </div>
      {operation.summary && <div className="agent-approval-summary selectable">{operation.summary}</div>}
      <EntityCards entities={entities} onNavigate={onNavigate} />
      {(Array.isArray(operation.affected_refs) || operation.risk) && (
        <div className="agent-approval-impact selectable">
          {Array.isArray(operation.affected_refs) && <span>{t("askferry:approval.affected", { n: operation.affected_refs.length })}</span>}
          {operation.risk && <span>{t("askferry:approval.risk", { risk: operation.risk })}</span>}
        </div>
      )}
      {failed && item.error && <div className="agent-approval-error mono selectable">{item.error}</div>}
      {applied && item.result?.saved_as && <div className="agent-approval-path mono selectable">{item.result.saved_as}</div>}
      {active && (
        <div className="agent-approval-actions">
          <button className="fbtn agent-approval-reject" onClick={onDismiss}>{t("askferry:approval.reject")}</button>
          <button className="fbtn fbtn-primary" onClick={approve}>{t("askferry:approval.approve")}</button>
        </div>
      )}
    </section>
  );
}
