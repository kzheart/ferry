import type { OrganizerInput, OrganizerResult } from "./organizer.js";
import { ProtocolError } from "./protocol.js";

export type OrganizationEngineMethod =
  | "session_backbone"
  | "session_summaries_set"
  | "organization_digest_context"
  | "organization_proposals_list"
  | "organization_propose";

export interface OrganizationEnginePort {
  invoke(
    method: OrganizationEngineMethod,
    params: Record<string, unknown>,
    workflowId: string,
  ): Promise<unknown>;
}

interface SessionInput {
  tool: string;
  id: string;
  ref: string;
  title?: string;
  project?: string;
  updated_at?: string;
}

interface Backbone {
  tool: string;
  id: string;
  fingerprint: string;
  pending?: string[];
  pending_sources?: Array<{ hash: string; text: string }>;
  segments: Array<{ hash: string; digest?: string | null }>;
}

function requiredText(value: unknown, field: string, max: number) {
  if (typeof value !== "string" || !value.trim() || value.length > max) {
    throw new ProtocolError("invalid_params", `${field} is invalid`);
  }
  return value.trim();
}

function parseInput(value: unknown): {
  sessions: SessionInput[];
  locale?: string;
} {
  if (
    typeof value !== "object" ||
    value === null ||
    !Array.isArray((value as { sessions?: unknown }).sessions)
  ) {
    throw new ProtocolError("invalid_params", "sessions must be an array");
  }
  const raw = (value as { sessions: unknown[]; locale?: unknown }).sessions;
  if (raw.length === 0 || raw.length > 50) {
    throw new ProtocolError("invalid_params", "sessions count is invalid");
  }
  const sessions = raw.map((item, index): SessionInput => {
    if (typeof item !== "object" || item === null) {
      throw new ProtocolError(
        "invalid_params",
        `sessions[${index}] is invalid`,
      );
    }
    const record = item as Record<string, unknown>;
    return {
      tool: requiredText(record.tool, "session.tool", 64),
      id: requiredText(record.id, "session.id", 512),
      ref: requiredText(record.ref, "session.ref", 512),
      ...(typeof record.title === "string"
        ? { title: record.title.slice(0, 500) }
        : {}),
      ...(typeof record.project === "string"
        ? { project: record.project.slice(0, 500) }
        : {}),
      ...(typeof record.updated_at === "string"
        ? { updated_at: record.updated_at.slice(0, 128) }
        : {}),
    };
  });
  const identities = new Set(sessions.map(({ tool, id }) => `${tool}\0${id}`));
  if (identities.size !== sessions.length) {
    throw new ProtocolError("invalid_params", "sessions must be unique");
  }
  const locale = (value as { locale?: unknown }).locale;
  return {
    sessions,
    ...(typeof locale === "string" && locale.trim()
      ? { locale: locale.trim().slice(0, 32) }
      : {}),
  };
}

function object(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ProtocolError("organization_failed", `${field} is invalid`);
  }
  return value as Record<string, unknown>;
}

function bounded(backbones: Backbone[]) {
  let chars = 0;
  let segments = 0;
  return backbones.filter((backbone) => {
    const pending = new Map(
      (backbone.pending_sources ?? []).map((item) => [item.hash, item.text]),
    );
    const nextChars = backbone.segments.reduce(
      (sum, segment) =>
        sum +
        (segment.digest?.length ?? pending.get(segment.hash)?.length ?? 0),
      0,
    );
    const fits =
      chars + nextChars <= 47_000 && segments + backbone.segments.length <= 190;
    if (fits) {
      chars += nextChars;
      segments += backbone.segments.length;
    }
    return fits;
  });
}

function existingProposal(proposals: unknown, backbones: Backbone[]) {
  if (!Array.isArray(proposals)) return null;
  const expected = new Map(
    backbones.map((item) => [`${item.tool}\0${item.id}`, item.fingerprint]),
  );
  return (
    proposals.find((candidate) => {
      const proposal = object(candidate, "proposal");
      if (
        proposal.status === "stale" ||
        !Array.isArray(proposal.targets) ||
        proposal.targets.length !== expected.size
      )
        return false;
      return proposal.targets.every((item) => {
        const target = object(item, "proposal target");
        return (
          expected.get(`${String(target.tool)}\0${String(target.id)}`) ===
          target.fingerprint
        );
      });
    }) ?? null
  );
}

function generationInput(
  backbones: Backbone[],
  sessions: SessionInput[],
  locale?: string,
): OrganizerInput {
  const sources = new Map(
    sessions.map((session) => [`${session.tool}\0${session.id}`, session]),
  );
  return {
    ...(locale ? { locale } : {}),
    sessions: backbones.map((backbone) => {
      const source = sources.get(`${backbone.tool}\0${backbone.id}`);
      const pending = new Map(
        (backbone.pending_sources ?? []).map((item) => [item.hash, item.text]),
      );
      return {
        tool: backbone.tool,
        id: backbone.id,
        ...(source?.title ? { title: source.title } : {}),
        ...(source?.project ? { project: source.project } : {}),
        ...(source?.updated_at ? { updated_at: source.updated_at } : {}),
        segments: backbone.segments
          .map((segment) => ({
            hash: segment.hash,
            text: (pending.get(segment.hash) ?? segment.digest ?? "").slice(
              0,
              24_000,
            ),
            ...(segment.digest ? { digest: segment.digest } : {}),
          }))
          .filter((segment) => segment.text),
      };
    }),
  };
}

function proposalTargets(context: unknown, generated: OrganizerResult) {
  const sessions = object(context, "organization context").sessions;
  if (!Array.isArray(sessions)) {
    throw new ProtocolError("organization_failed", "context sessions missing");
  }
  const results = new Map(
    generated.sessions.map((item) => [`${item.tool}\0${item.id}`, item]),
  );
  const clusters = new Map<string, { id: string; name: string }>();
  for (const cluster of generated.clusters) {
    for (const member of cluster.members) {
      clusters.set(`${member.tool}\0${member.id}`, cluster);
    }
  }
  return sessions.map((item) => {
    const session = object(item, "organization context session");
    const key = `${String(session.tool)}\0${String(session.id)}`;
    const result = results.get(key);
    if (!result) {
      throw new ProtocolError(
        "organization_failed",
        `organizer omitted ${key}`,
      );
    }
    const segments = Array.isArray(session.segments) ? session.segments : [];
    const cluster = clusters.get(key);
    return {
      tool: session.tool,
      id: session.id,
      fingerprint: session.fingerprint,
      sources: segments.map((segment) => object(segment, "segment").hash),
      suggested: {
        name: result.title,
        summary: result.summary,
        tags: result.tags,
        dead_candidate: result.dead,
        ...(result.dead_reason ? { dead_reason: result.dead_reason } : {}),
        ...(cluster
          ? { cluster_id: cluster.id, cluster_name: cluster.name }
          : {}),
      },
    };
  });
}

export async function runOrganizationWorkflow(
  rawInput: unknown,
  workflowId: string,
  engine: OrganizationEnginePort,
  generate: (input: OrganizerInput) => Promise<OrganizerResult>,
) {
  const input = parseInput(rawInput);
  const backbones = await Promise.all(
    input.sessions.map(async (session): Promise<Backbone> => {
      const result = object(
        await engine.invoke(
          "session_backbone",
          { tool: session.tool, ref: session.ref },
          workflowId,
        ),
        "session backbone",
      );
      if (!Array.isArray(result.segments)) {
        throw new ProtocolError(
          "organization_failed",
          "session backbone segments missing",
        );
      }
      return {
        ...result,
        tool: session.tool,
        id: session.id,
      } as unknown as Backbone;
    }),
  );
  const selected = bounded(backbones);
  if (!selected.length) {
    throw new ProtocolError(
      "organization_failed",
      "no session segment fits the organizer budget",
    );
  }
  const proposals = await engine.invoke(
    "organization_proposals_list",
    {},
    workflowId,
  );
  const existing = existingProposal(proposals, selected);
  if (existing) return existing;

  const selectedKeys = new Set(
    selected.map((item) => `${item.tool}\0${item.id}`),
  );
  const selectedSessions = input.sessions.filter((item) =>
    selectedKeys.has(`${item.tool}\0${item.id}`),
  );
  const generated = await generate(
    generationInput(selected, selectedSessions, input.locale),
  );
  await Promise.all(
    selected.map(async (backbone) => {
      const pending = new Set(backbone.pending ?? []);
      const result = generated.sessions.find(
        (item) => item.tool === backbone.tool && item.id === backbone.id,
      );
      const digests = Object.fromEntries(
        Object.entries(result?.digests ?? {}).filter(([hash]) =>
          pending.has(hash),
        ),
      );
      if (Object.keys(digests).length) {
        await engine.invoke(
          "session_summaries_set",
          { tool: backbone.tool, id: backbone.id, digests },
          workflowId,
        );
      }
    }),
  );
  const context = await engine.invoke(
    "organization_digest_context",
    {
      targets: selected.map(({ tool, id }) => ({ tool, id })),
    },
    workflowId,
  );
  return engine.invoke(
    "organization_propose",
    { targets: proposalTargets(context, generated) },
    workflowId,
  );
}
