export const FERRY_ENTITY = Object.freeze({
  session: "Session",
  migration: "Migration",
  edit: "Edit",
  usage: "UsageSlice",
});

const isObject = value => value != null && typeof value === "object" && !Array.isArray(value);
const text = value => typeof value === "string" && value ? value : undefined;

function sessionEntity(value) {
  if (!isObject(value)) return null;
  const ref = text(value.ref) || text(value.session_ref);
  const id = text(value.session_id) || text(value.id);
  if (!ref && !id) return null;
  return {
    type: FERRY_ENTITY.session,
    key: `session:${value.tool || "unknown"}:${ref || id}:${value.locator || value.turn || ""}`,
    tool: text(value.tool) || text(value.agent),
    ref,
    sessionId: id,
    title: text(value.title),
    project: text(value.project),
    updated: value.updated,
    model: text(value.model),
    messageCount: value.message_count,
    locator: text(value.locator),
    turn: value.turn ?? value.round_index,
    raw: value,
  };
}

function migrationEntity(value) {
  if (!isObject(value)) return null;
  const preview = isObject(value.preview) ? value.preview : value;
  const applied = isObject(value.result?.result) ? value.result.result : value.result;
  const ref = text(value.ref) || text(preview.ref) ||
    (Array.isArray(value.affected_refs) ? text(value.affected_refs[0]) : undefined);
  if (!ref && !value.operation_id && !value.migration_id && !value.saved_as) return null;
  return {
    type: FERRY_ENTITY.migration,
    key: `migration:${value.operation_id || value.migration_id || value.saved_as || ref}`,
    id: text(value.operation_id) || text(value.migration_id),
    ref,
    sourceTool: text(value.source_tool) || text(preview.source_tool),
    targetTool: text(value.target_tool) || text(preview.target_tool),
    sourceSessionId: text(value.source_session_id) || text(preview.source_session_id),
    targetSessionId: text(value.target_session_id) || text(value.session_id) ||
      text(applied?.session_id),
    savedAs: text(value.saved_as) || text(applied?.saved_as),
    preview,
    status: text(value.status),
    raw: value,
  };
}

function editEntity(value) {
  if (!isObject(value)) return null;
  const preview = isObject(value.preview) ? value.preview : value;
  const applied = isObject(value.result?.result) ? value.result.result : value.result;
  const ref = text(value.ref) || text(preview.ref) ||
    (Array.isArray(value.affected_refs) ? text(value.affected_refs[0]) : undefined);
  if (!ref && !value.operation_id && !value.edit_id) return null;
  const changes = Array.isArray(preview.changes) ? preview.changes : [];
  return {
    type: FERRY_ENTITY.edit,
    key: `edit:${value.operation_id || value.edit_id || ref}`,
    id: text(value.operation_id) || text(value.edit_id),
    tool: text(value.tool) || text(preview.tool),
    ref,
    sessionId: text(value.session_id) || text(preview.session_id) ||
      text(applied?.session_id) || text(applied?.session?.session_id),
    preview,
    changes,
    locators: changes.map(change => change?.locator).filter(Boolean),
    turn: changes.find(change => change?.turn != null)?.turn,
    status: text(value.status),
    raw: value,
  };
}

function usageEntity(value, args = {}) {
  if (!isObject(value) || !isObject(value.tokens)) return null;
  const filters = isObject(value.filters) ? value.filters : args;
  return {
    type: FERRY_ENTITY.usage,
    key: `usage:${JSON.stringify(filters.time_range || "all")}:${JSON.stringify(filters.agents || "all")}:${JSON.stringify(filters.projects || "all")}`,
    timeRange: filters.time_range,
    agents: filters.agents,
    projects: filters.projects,
    sessions: value.sessions || 0,
    tokens: value.tokens,
    byAgent: isObject(value.by_agent) ? value.by_agent : {},
    cost: value.cost,
    currency: text(value.currency) || "USD",
    raw: value,
  };
}

function explicitEntity(value) {
  if (!isObject(value)) return null;
  const kind = value.type || value.entity_type || value.entity;
  if (kind === FERRY_ENTITY.session || kind === "session") return sessionEntity(value);
  if (kind === FERRY_ENTITY.migration || kind === "migration") return migrationEntity(value);
  if (kind === FERRY_ENTITY.edit || kind === "edit") return editEntity(value);
  if (kind === FERRY_ENTITY.usage || kind === "usage") return usageEntity(value, value.filters);
  return null;
}

export function entitiesFromToolResult(name, result, args = {}) {
  const rawDetails = result?.details;
  const details = isObject(rawDetails?.operation)
    ? { ...rawDetails.operation, status: rawDetails.status,
        result: rawDetails.result }
    : rawDetails;
  if (!isObject(details)) return [];
  const declared = Array.isArray(details.entities)
    ? details.entities.map(explicitEntity).filter(Boolean)
    : [];
  if (declared.length) return declared;

  switch (name) {
    case "session_search":
      return (details.sessions || []).map(sessionEntity).filter(Boolean);
    case "session_read": {
      const root = sessionEntity(details);
      const matches = (details.matches || details.messages || [])
        .map(item => sessionEntity({ ...item, tool: details.tool, ref: details.ref,
          session_id: details.session_id, title: details.title }))
        .filter(entity => entity?.locator);
      return root ? [root, ...matches] : matches;
    }
    case "usage": {
      const entity = usageEntity(details, args);
      return entity ? [entity] : [];
    }
    case "migrate": {
      const entity = migrationEntity(details);
      return entity ? [entity] : [];
    }
    case "session_edit": {
      const kind = details.kind || details.mode;
      const entity = kind === "migration" ? migrationEntity(details) : editEntity(details);
      return entity ? [entity] : [];
    }
    default:
      return [];
  }
}

export function navigationActionFor(entity) {
  if (!entity) return null;
  switch (entity.type) {
    case FERRY_ENTITY.session:
      return { view: "library", sessionId: entity.sessionId, ref: entity.ref,
        tool: entity.tool, locator: entity.locator, turn: entity.turn };
    case FERRY_ENTITY.migration:
      return entity.targetSessionId || entity.savedAs
        ? { view: "library", sessionId: entity.targetSessionId,
            ref: entity.savedAs, tool: entity.targetTool }
        : { view: "history", migrationId: entity.id, ref: entity.ref };
    case FERRY_ENTITY.edit:
      return { view: "library", sessionId: entity.sessionId, ref: entity.ref,
        tool: entity.tool, locator: entity.locators[0],
        ...(entity.turn == null ? {} : { turn: entity.turn }) };
    case FERRY_ENTITY.usage:
      return { view: "overview", timeRange: entity.timeRange,
        agents: entity.agents, projects: entity.projects };
    default:
      return null;
  }
}

export function rendererForEntity(entity) {
  return {
    [FERRY_ENTITY.session]: "session-card",
    [FERRY_ENTITY.migration]: "migration-preview",
    [FERRY_ENTITY.edit]: "edit-diff",
    [FERRY_ENTITY.usage]: "usage-slice",
  }[entity?.type] || null;
}
