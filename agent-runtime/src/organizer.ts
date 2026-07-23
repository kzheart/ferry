import { ProtocolError } from "./protocol.js";

const MAX_SESSIONS = 50;
const MAX_SEGMENTS = 200;
const MAX_SOURCE_CHARS = 48_000;

export interface OrganizerSegment {
  hash: string;
  text: string;
  digest?: string | null;
}

export interface OrganizerSession {
  tool: string;
  id: string;
  title?: string | null;
  project?: string | null;
  updated_at?: string | null;
  segments: OrganizerSegment[];
}

export interface OrganizerInput {
  sessions: OrganizerSession[];
  locale?: string;
}

export interface OrganizerSessionResult {
  tool: string;
  id: string;
  digests: Record<string, string>;
  title: string;
  summary: string;
  tags: string[];
  dead: boolean;
  dead_reason?: string;
}

export interface OrganizerResult {
  sessions: OrganizerSessionResult[];
  clusters: Array<{
    id: string;
    name: string;
    members: Array<{ tool: string; id: string }>;
  }>;
}

function requiredText(value: unknown, field: string, max: number) {
  if (typeof value !== "string" || !value.trim() || value.length > max) {
    throw new ProtocolError("invalid_params", `${field} is invalid`);
  }
  return value.trim();
}

export function parseOrganizerInput(value: unknown): OrganizerInput {
  if (
    typeof value !== "object" ||
    value === null ||
    !Array.isArray((value as { sessions?: unknown }).sessions)
  ) {
    throw new ProtocolError("invalid_params", "sessions must be an array");
  }
  const raw = (value as { sessions: unknown[]; locale?: unknown }).sessions;
  if (raw.length === 0 || raw.length > MAX_SESSIONS) {
    throw new ProtocolError("invalid_params", "sessions count is invalid");
  }
  let segmentCount = 0;
  let sourceChars = 0;
  const sessions = raw.map((item, sessionIndex): OrganizerSession => {
    if (typeof item !== "object" || item === null) {
      throw new ProtocolError(
        "invalid_params",
        `sessions[${sessionIndex}] is invalid`,
      );
    }
    const record = item as Record<string, unknown>;
    if (!Array.isArray(record.segments)) {
      throw new ProtocolError(
        "invalid_params",
        `sessions[${sessionIndex}].segments must be an array`,
      );
    }
    const segments = record.segments.map((segment, segmentIndex) => {
      if (typeof segment !== "object" || segment === null) {
        throw new ProtocolError(
          "invalid_params",
          `segments[${segmentIndex}] is invalid`,
        );
      }
      const data = segment as Record<string, unknown>;
      const text = requiredText(data.text, "segment.text", 24_000);
      sourceChars += text.length;
      segmentCount += 1;
      return {
        hash: requiredText(data.hash, "segment.hash", 128),
        text,
        ...(typeof data.digest === "string" && data.digest.trim()
          ? { digest: data.digest.trim().slice(0, 4_000) }
          : {}),
      };
    });
    return {
      tool: requiredText(record.tool, "session.tool", 64),
      id: requiredText(record.id, "session.id", 512),
      ...(typeof record.title === "string"
        ? { title: record.title.slice(0, 500) }
        : {}),
      ...(typeof record.project === "string"
        ? { project: record.project.slice(0, 500) }
        : {}),
      ...(typeof record.updated_at === "string"
        ? { updated_at: record.updated_at.slice(0, 128) }
        : {}),
      segments,
    };
  });
  if (
    segmentCount === 0 ||
    segmentCount > MAX_SEGMENTS ||
    sourceChars > MAX_SOURCE_CHARS
  ) {
    throw new ProtocolError("invalid_params", "organizer input is too large");
  }
  const localeValue = (value as { locale?: unknown }).locale;
  return {
    sessions,
    ...(typeof localeValue === "string" && localeValue.trim()
      ? { locale: localeValue.trim().slice(0, 32) }
      : {}),
  };
}

export function organizerPrompt(input: OrganizerInput) {
  const payload = input.sessions.map((session) => ({
    tool: session.tool,
    id: session.id,
    title: session.title ?? null,
    project: session.project ?? null,
    updated_at: session.updated_at ?? null,
    segments: session.segments.map((segment) => ({
      hash: segment.hash,
      text: segment.text,
      digest: segment.digest ?? null,
    })),
  }));
  return [
    "You organize a private local archive of AI coding sessions.",
    "Return only one JSON object. Do not wrap it in markdown.",
    `Write user-facing text in ${input.locale ?? "the source language"}.`,
    "For every segment without a digest, write one concise digest supported only by that segment. When digest is already provided, copy it byte-for-byte unchanged. Never invent a result, decision, file, command, or status.",
    "For every session, propose a short title, a factual summary, 1-5 useful tags, and whether it is a dead/abandoned session. Mark dead only when the transcript itself clearly shows abandonment, supersession, or no actionable outcome.",
    "Cluster related sessions across agents by durable topic or project. A session may belong to at most one cluster; omit singleton clusters.",
    'Schema: {"sessions":[{"tool":"...","id":"...","digests":{"sha256:...":"..."},"title":"...","summary":"...","tags":["..."],"dead":false,"dead_reason":"..."}],"clusters":[{"id":"stable-slug","name":"...","members":[{"tool":"...","id":"..."}]}]}',
    "Input:",
    JSON.stringify(payload),
  ].join("\n");
}

function jsonObject(text: string): unknown {
  const trimmed = text.trim();
  const unwrapped = trimmed
    .replace(/^```(?:json)?\s*/i, "")
    .replace(/\s*```$/, "");
  try {
    return JSON.parse(unwrapped);
  } catch {
    throw new ProtocolError(
      "organizer_invalid_response",
      "organizer returned invalid JSON",
    );
  }
}

export function validateOrganizerResult(
  text: string,
  input: OrganizerInput,
): OrganizerResult {
  const value = jsonObject(text);
  if (typeof value !== "object" || value === null) {
    throw new ProtocolError(
      "organizer_invalid_response",
      "organizer result must be an object",
    );
  }
  const result = value as Record<string, unknown>;
  if (!Array.isArray(result.sessions) || !Array.isArray(result.clusters)) {
    throw new ProtocolError(
      "organizer_invalid_response",
      "organizer result is incomplete",
    );
  }
  const expected = new Map(
    input.sessions.map((session) => [
      `${session.tool}\0${session.id}`,
      new Set(session.segments.map((segment) => segment.hash)),
    ]),
  );
  const cached = new Map(
    input.sessions.flatMap((session) =>
      session.segments
        .filter(
          (segment): segment is OrganizerSegment & { digest: string } =>
            typeof segment.digest === "string" && !!segment.digest,
        )
        .map(
          (segment) =>
            [
              `${session.tool}\0${session.id}\0${segment.hash}`,
              segment.digest,
            ] as const,
        ),
    ),
  );
  const sessions = result.sessions.map((item): OrganizerSessionResult => {
    if (typeof item !== "object" || item === null) {
      throw new ProtocolError(
        "organizer_invalid_response",
        "organizer session is invalid",
      );
    }
    const record = item as Record<string, unknown>;
    const tool = requiredText(record.tool, "result.tool", 64);
    const id = requiredText(record.id, "result.id", 512);
    const hashes = expected.get(`${tool}\0${id}`);
    if (!hashes || typeof record.digests !== "object" || !record.digests) {
      throw new ProtocolError(
        "organizer_invalid_response",
        "organizer session does not match input",
      );
    }
    const digests: Record<string, string> = {};
    for (const [hash, digest] of Object.entries(record.digests)) {
      if (!hashes.has(hash) || typeof digest !== "string" || !digest.trim()) {
        throw new ProtocolError(
          "organizer_invalid_response",
          "organizer digest does not match a source segment",
        );
      }
      digests[hash] = digest.trim().slice(0, 4_000);
      const prior = cached.get(`${tool}\0${id}\0${hash}`);
      if (prior !== undefined && digests[hash] !== prior) {
        throw new ProtocolError(
          "organizer_invalid_response",
          "organizer changed a cached digest",
        );
      }
    }
    if ([...hashes].some((hash) => !digests[hash])) {
      throw new ProtocolError(
        "organizer_invalid_response",
        "organizer omitted a segment digest",
      );
    }
    const tags = Array.isArray(record.tags)
      ? record.tags
          .filter((tag): tag is string => typeof tag === "string")
          .map((tag) => tag.trim())
          .filter(Boolean)
          .slice(0, 5)
      : [];
    return {
      tool,
      id,
      digests,
      title: requiredText(record.title, "result.title", 200),
      summary: requiredText(record.summary, "result.summary", 4_000),
      tags,
      dead: record.dead === true,
      ...(record.dead === true && typeof record.dead_reason === "string"
        ? { dead_reason: record.dead_reason.trim().slice(0, 1_000) }
        : {}),
    };
  });
  if (sessions.length !== expected.size) {
    throw new ProtocolError(
      "organizer_invalid_response",
      "organizer omitted a session",
    );
  }
  const validMembers = new Set(expected.keys());
  const clusters = result.clusters.map((item) => {
    if (typeof item !== "object" || item === null) {
      throw new ProtocolError(
        "organizer_invalid_response",
        "organizer cluster is invalid",
      );
    }
    const record = item as Record<string, unknown>;
    if (!Array.isArray(record.members)) {
      throw new ProtocolError(
        "organizer_invalid_response",
        "cluster members are invalid",
      );
    }
    const members = record.members.map((member) => {
      if (typeof member !== "object" || member === null) {
        throw new ProtocolError(
          "organizer_invalid_response",
          "cluster member is invalid",
        );
      }
      const memberRecord = member as Record<string, unknown>;
      const tool = requiredText(memberRecord.tool, "member.tool", 64);
      const id = requiredText(memberRecord.id, "member.id", 512);
      if (!validMembers.has(`${tool}\0${id}`)) {
        throw new ProtocolError(
          "organizer_invalid_response",
          "cluster member is not in the input",
        );
      }
      return { tool, id };
    });
    return {
      id: requiredText(record.id, "cluster.id", 128),
      name: requiredText(record.name, "cluster.name", 200),
      members,
    };
  });
  return { sessions, clusters };
}
