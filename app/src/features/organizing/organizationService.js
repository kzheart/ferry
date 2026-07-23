import { agentCommand } from "../../api/agent/agentClient.js";
import { rpc } from "../../api/transport/rpc.js";
import { sessionRef } from "../../domain/sessions/sessionModel.js";
import {
  existingProposalFor,
  generationInput,
  proposalTargets,
} from "../../domain/organizing/organizationModel.js";

const running = new Map();

function bounded(backbones) {
  let chars = 0;
  let segments = 0;
  return backbones.filter(backbone => {
    const nextChars = backbone.segments.reduce((sum, segment) =>
      sum + (segment.digest?.length || backbone.pending_sources
        ?.find(item => item.hash === segment.hash)?.text?.length || 0), 0);
    const fits = chars + nextChars <= 47_000 &&
      segments + backbone.segments.length <= 190;
    if (fits) {
      chars += nextChars;
      segments += backbone.segments.length;
    }
    return fits;
  });
}

async function generate(sessions, locale) {
  const targets = sessions.slice(0, 50);
  const backbones = await Promise.all(targets.map(async session => ({
    tool: session.tool,
    id: session.id,
    ...await rpc("session_backbone", {
      tool: session.tool, ref: sessionRef(session),
    }),
  })));
  const selectedBackbones = bounded(backbones);
  if (!selectedBackbones.length) throw new Error("No session segment fits the organizer budget");
  const selectedKeys = new Set(selectedBackbones.map(item => `${item.tool}\0${item.id}`));
  const selectedSessions = targets.filter(item => selectedKeys.has(`${item.tool}\0${item.id}`));
  const proposals = await rpc("organization_proposals_list", {});
  const existing = existingProposalFor(proposals, selectedBackbones);
  if (existing) return existing;

  const generated = await agentCommand("organization.generate",
    generationInput(selectedBackbones, selectedSessions, locale));
  await Promise.all(selectedBackbones.map(async backbone => {
    const pending = new Set(backbone.pending || []);
    const result = generated.sessions.find(item =>
      item.tool === backbone.tool && item.id === backbone.id);
    const digests = Object.fromEntries(Object.entries(result?.digests || {})
      .filter(([hash]) => pending.has(hash)));
    if (Object.keys(digests).length) {
      await rpc("session_summaries_set", {
        tool: backbone.tool, id: backbone.id, digests,
      });
    }
  }));
  const context = await rpc("organization_digest_context", {
    targets: selectedBackbones.map(item => ({ tool: item.tool, id: item.id })),
  });
  return rpc("organization_propose", {
    targets: proposalTargets(context, generated),
  });
}

export function generateOrganizationProposal(sessions, locale) {
  const key = sessions.map(session => `${session.tool}:${session.id}`).sort().join("|");
  if (!running.has(key)) {
    const task = generate(sessions, locale).finally(() => running.delete(key));
    running.set(key, task);
  }
  return running.get(key);
}
