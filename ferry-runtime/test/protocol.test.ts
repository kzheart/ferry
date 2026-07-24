import { describe, expect, it } from "vitest";
import { dispatch } from "../src/application/command-router.js";
import {
  parseCommand,
  PROTOCOL_VERSION,
  ProtocolError,
} from "../src/protocol/messages.js";
import { AgentRuntime } from "../src/application/runtime.js";
import { createProtocolTestBackend } from "./test-backend.js";

describe("JSONL protocol", () => {
  it("rejects unknown protocol versions and methods", () => {
    expect(() =>
      parseCommand({ protocol: "v0", id: "1", method: "health", params: {} }),
    ).toThrow(ProtocolError);
    expect(() =>
      parseCommand({
        protocol: PROTOCOL_VERSION,
        id: "1",
        method: "shell",
        params: {},
      }),
    ).toThrow(ProtocolError);
    expect(() =>
      parseCommand({
        protocol: PROTOCOL_VERSION,
        id: "1",
        method: "health",
      }),
    ).toThrow(ProtocolError);
    expect(() =>
      parseCommand({
        protocol: PROTOCOL_VERSION,
        id: "1",
        method: "health",
        params: {},
        legacy: true,
      }),
    ).toThrow(ProtocolError);
  });

  it("returns typed health and validation responses", async () => {
    const runtime = await AgentRuntime.create({
      backendFactory: createProtocolTestBackend,
    });
    const health = await dispatch(
      runtime,
      parseCommand({
        protocol: PROTOCOL_VERSION,
        id: "1",
        method: "health",
        params: {},
      }),
    );
    expect(health).toMatchObject({
      ok: true,
      result: {
        status: "ready",
        service: "ferry-runtime",
        contract_hash: expect.stringMatching(/^sha256:/),
        pi_version: "0.81.1",
        provider: "protocol-test",
        model: "protocol-test-driver",
        credential: "available",
      },
    });

    const invalid = await dispatch(
      runtime,
      parseCommand({
        protocol: PROTOCOL_VERSION,
        id: "2",
        method: "events.replay",
        params: { session_id: "x" },
      }),
    );
    expect(invalid).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
  });

  it("exposes role CRUD and resolves role_id during session.create", async () => {
    const runtime = await AgentRuntime.create({
      backendFactory: createProtocolTestBackend,
    });
    const role = {
      id: "reader",
      name: "Reader",
      persona: "Read only.",
      tools: ["session_search", "session_read"],
      allow_bash: false,
      apply_policy: "manual",
    };
    const created = await dispatch(
      runtime,
      parseCommand({
        protocol: PROTOCOL_VERSION,
        id: "role-create",
        method: "role.create",
        params: { role },
      }),
    );
    expect(created).toMatchObject({
      ok: true,
      result: { id: "reader", builtin: false },
    });

    const session = await dispatch(
      runtime,
      parseCommand({
        protocol: PROTOCOL_VERSION,
        id: "session-create",
        method: "session.create",
        params: { session_id: "s1", role_id: "reader" },
      }),
    );
    expect(session).toMatchObject({
      ok: true,
      result: { session_id: "s1", role_id: "reader" },
    });

    const denied = await dispatch(
      runtime,
      parseCommand({
        protocol: PROTOCOL_VERSION,
        id: "role-delete",
        method: "role.delete",
        params: { role_id: "default" },
      }),
    );
    expect(denied).toMatchObject({
      ok: false,
      error: { code: "invalid_role" },
    });
  });
});
