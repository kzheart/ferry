import { describe, expect, it } from "vitest";
import { dispatch } from "../src/commands.js";
import {
  parseCommand,
  PROTOCOL_VERSION,
  ProtocolError,
} from "../src/protocol.js";
import { AgentRuntime } from "../src/runtime.js";
import { createProtocolTestBackend } from "./test-backend.js";

describe("JSONL protocol", () => {
  it("rejects unknown protocol versions and methods", () => {
    expect(() =>
      parseCommand({ protocol: "v0", id: "1", method: "health" }),
    ).toThrow(ProtocolError);
    expect(() =>
      parseCommand({ protocol: PROTOCOL_VERSION, id: "1", method: "shell" }),
    ).toThrow(ProtocolError);
  });

  it("returns typed health and validation responses", async () => {
    const runtime = await AgentRuntime.create({
      backendFactory: createProtocolTestBackend,
    });
    const health = await dispatch(
      runtime,
      parseCommand({ protocol: PROTOCOL_VERSION, id: "1", method: "health" }),
    );
    expect(health).toMatchObject({
      ok: true,
      result: {
        status: "ok",
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
});
