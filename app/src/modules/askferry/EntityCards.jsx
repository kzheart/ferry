import { TOOL_NAME } from "../../shared/contracts/tools.js";
import { renderEvents } from "../../shared/contracts/events.js";
import { ToolIcon } from "../../shared/ui/icons.jsx";
import { LossCols } from "../../shared/ui/primitives.jsx";
import { navigationActionFor, rendererForEntity }
  from "./ferryEntities.js";
import { useTranslation } from "react-i18next";

const shell = {
  border: "1px solid var(--line3)", borderRadius: 10, padding: "10px 12px",
  background: "var(--surface)", display: "flex", gap: 10, alignItems: "center",
  width: "100%", textAlign: "left", color: "var(--tx1)", cursor: "default",
};
const secondary = { fontSize: 11, color: "var(--tx4)", marginTop: 3 };

const totalTokens = tokens => Object.values(tokens || {})
  .filter(value => typeof value === "number")
  .reduce((sum, value) => sum + value, 0);

function SessionCard({ entity, onNavigate }) {
  const { t } = useTranslation();
  return (
    <button type="button" style={shell}
      onClick={() => onNavigate?.(navigationActionFor(entity), entity)}>
      <ToolIcon tool={entity.tool} size={28} />
      <span style={{ flex: 1, minWidth: 0 }}>
        <span style={{ display: "block", fontSize: 12.5, fontWeight: 600,
          overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {entity.title || entity.sessionId || entity.ref}
        </span>
        <span style={secondary}>
          {[entity.project,
            entity.messageCount != null ? t("askferry:entity.messages", { n: entity.messageCount }) : null,
            entity.turn != null ? t("askferry:entity.turn", { n: entity.turn }) : null]
            .filter(Boolean).join(" · ")}
        </span>
      </span>
      <span aria-hidden style={{ color: "var(--tx5)" }}>›</span>
    </button>
  );
}

function MigrationCard({ entity, onNavigate }) {
  const { t } = useTranslation();
  const loss = entity.preview?.loss || {};
  const lossCount = Object.values(loss).filter(value =>
    typeof value === "number").reduce((sum, value) => sum + value, 0);
  return (
    <div role="button" tabIndex={0} style={{ ...shell, display: "block", padding: 0,
      overflow: "hidden" }}
      onClick={() => onNavigate?.(navigationActionFor(entity), entity)}
      onKeyDown={event => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onNavigate?.(navigationActionFor(entity), entity);
        }
      }}>
      <span style={{ display: "flex", gap: 10, alignItems: "flex-start", padding: "10px 12px" }}>
        <span style={{ width: 8, height: 8, borderRadius: "50%", marginTop: 5,
          background: lossCount ? "var(--warn)" : "var(--ok)", flex: "none" }} />
        <span style={{ flex: 1, minWidth: 0 }}>
          <span style={{ display: "block", fontSize: 12.5, fontWeight: 600 }}>
            {TOOL_NAME[entity.sourceTool] || entity.sourceTool || t("askferry:entity.session")} →{" "}
            {TOOL_NAME[entity.targetTool] || entity.targetTool || t("askferry:entity.migration")}
          </span>
          <span style={secondary}>
            {entity.preview?.message_count != null
              ? t("askferry:entity.messages", { n: entity.preview.message_count })
              : t("askferry:entity.migrationResult")}
          </span>
        </span>
        <span aria-hidden style={{ color: "var(--tx5)" }}>›</span>
      </span>
      {entity.preview?.loss && <LossCols loss={entity.preview.loss} compact />}
    </div>
  );
}

function EditCard({ entity, onNavigate }) {
  const { t } = useTranslation();
  const before = entity.preview?.before;
  const after = entity.preview?.after;
  const changes = renderEvents(entity.changes).slice(0, 3);
  return (
    <button type="button" style={{ ...shell, alignItems: "flex-start" }}
      onClick={() => onNavigate?.(navigationActionFor(entity), entity)}>
      <span style={{ fontFamily: "monospace", fontSize: 13, color: "var(--warn-deep)",
        flex: "none" }}>±</span>
      <span style={{ flex: 1, minWidth: 0 }}>
        <span style={{ display: "block", fontSize: 12.5, fontWeight: 600 }}>
          {t("askferry:entity.edits", {
            count: entity.changes.length || 1, n: entity.changes.length || 1,
          })}
        </span>
        <span style={secondary}>
          {before?.size != null && after?.size != null
            ? `${before.size} → ${after.size} bytes`
            : entity.locators[0] || t("askferry:entity.openDiff")}
        </span>
        {changes.map((change, index) => (
          <span key={index} style={{ display: "block", fontSize: 11,
            color: "var(--tx3)", marginTop: 4 }}>{change}</span>
        ))}
      </span>
      <span aria-hidden style={{ color: "var(--tx5)" }}>›</span>
    </button>
  );
}

function UsageCard({ entity, onNavigate }) {
  const { t } = useTranslation();
  const total = totalTokens(entity.tokens);
  const agents = Object.keys(entity.byAgent);
  return (
    <button type="button" style={{ ...shell, alignItems: "flex-start" }}
      onClick={() => onNavigate?.(navigationActionFor(entity), entity)}>
      <span style={{ width: 28, height: 28, borderRadius: 7, background: "var(--acc-soft2)",
        color: "var(--acc)", display: "inline-flex", alignItems: "center",
        justifyContent: "center", fontSize: 12, flex: "none" }}>∿</span>
      <span style={{ flex: 1, minWidth: 0 }}>
        <span style={{ display: "block", fontSize: 12.5, fontWeight: 600 }}>
          {t("askferry:entity.tokens", { n: total.toLocaleString() })}
        </span>
        <span style={secondary}>
          {t("askferry:entity.sessions", { count: entity.sessions, n: entity.sessions })}
          {agents.length ? ` · ${agents.join(", ")}` : ""}
        </span>
      </span>
      <span aria-hidden style={{ color: "var(--tx5)" }}>›</span>
    </button>
  );
}

export function EntityCard({ entity, onNavigate }) {
  switch (rendererForEntity(entity)) {
    case "session-card":
      return <SessionCard entity={entity} onNavigate={onNavigate} />;
    case "migration-preview":
      return <MigrationCard entity={entity} onNavigate={onNavigate} />;
    case "edit-diff":
      return <EditCard entity={entity} onNavigate={onNavigate} />;
    case "usage-slice":
      return <UsageCard entity={entity} onNavigate={onNavigate} />;
    default:
      return null;
  }
}

export default function EntityCards({ entities = [], onNavigate }) {
  if (!entities.length) return null;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 7, marginTop: 7,
      maxWidth: 560 }}>
      {entities.map(entity => (
        <EntityCard key={entity.key} entity={entity} onNavigate={onNavigate} />
      ))}
    </div>
  );
}
