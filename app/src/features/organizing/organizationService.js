import { agentCommand } from "../../api/agent/agentClient.js";
import { sessionRef } from "../../domain/sessions/sessionModel.js";

export function generateOrganizationProposal(sessions, locale) {
  return agentCommand("organization.start", {
    locale,
    sessions: sessions.slice(0, 50).map(session => ({
      tool: session.tool,
      id: session.id,
      ref: sessionRef(session),
      title: session.title,
      project: session.project,
      ...(session.updated_at
        ? { updated_at: String(session.updated_at) }
        : {}),
    })),
  });
}
