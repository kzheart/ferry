import { describe, expect, it } from "vitest";

import { OrganizationCoordinator } from "../src/organizing/coordinator.js";
import type { OrganizerResult } from "../src/organizing/organizer.js";
import type { ProviderHost } from "../src/providers/provider-host.js";

const deferred = <T>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
};

const input = {
  sessions: [
    {
      tool: "codex",
      id: "session-a",
      ref: "fsr_session_a",
      title: "测试修复",
    },
  ],
};

const generated: OrganizerResult = {
  sessions: [
    {
      tool: "codex",
      id: "session-a",
      digests: { "sha256:a": "完成测试修复。" },
      title: "测试修复",
      summary: "修复并验证测试。",
      tags: ["测试"],
      dead: false,
    },
  ],
  clusters: [],
};

function coordinator(modelResult: Promise<OrganizerResult>) {
  let sequence = 0;
  const calls: string[] = [];
  const value = new OrganizationCoordinator({
    providerHost: {
      organize: () => modelResult,
    } as unknown as ProviderHost,
    newId: () => `fixture_${++sequence}`,
    invokeEngine: async (method) => {
      calls.push(method);
      if (method === "session_backbone") {
        return {
          fingerprint: "fingerprint-a",
          pending: ["sha256:a"],
          pending_sources: [{ hash: "sha256:a", text: "修复测试并完成验证。" }],
          segments: [{ hash: "sha256:a", digest: null }],
        };
      }
      if (method === "organization_proposals_list") return [];
      if (method === "session_summaries_set") return { updated: true };
      if (method === "organization_digest_context") {
        return {
          sessions: [
            {
              tool: "codex",
              id: "session-a",
              fingerprint: "fingerprint-a",
              segments: [{ hash: "sha256:a", digest: "完成测试修复。" }],
            },
          ],
        };
      }
      return { proposal_id: "proposal-a", status: "pending" };
    },
  });
  return { value, calls };
}

async function waitForTerminal(value: OrganizationCoordinator, jobId: string) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const state = value.status(jobId);
    if (state.status !== "running") return state;
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  throw new Error("organization job did not finish");
}

describe("organization jobs", () => {
  it("starts immediately, deduplicates, and exposes the completed result", async () => {
    const model = deferred<OrganizerResult>();
    const { value } = coordinator(model.promise);

    const started = value.start(input);
    expect(started).toMatchObject({ status: "running" });
    expect(value.start(input)).toEqual(started);

    model.resolve(generated);
    await expect(waitForTerminal(value, started.job_id)).resolves.toMatchObject(
      {
        status: "completed",
        result: { proposal_id: "proposal-a", status: "pending" },
      },
    );
  });

  it("cancels before the workflow enters its mutation phase", async () => {
    const model = deferred<OrganizerResult>();
    const { value, calls } = coordinator(model.promise);
    const started = value.start(input);

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(value.cancel(started.job_id)).toMatchObject({
      status: "cancelled",
      accepted: true,
    });
    model.resolve(generated);
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(value.status(started.job_id).status).toBe("cancelled");
    expect(calls).not.toContain("session_summaries_set");
    expect(calls).not.toContain("organization_propose");
  });
});
