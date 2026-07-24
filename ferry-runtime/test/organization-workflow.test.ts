import { describe, expect, it } from "vitest";
import {
  runOrganizationWorkflow,
  type OrganizationEngineMethod,
} from "../src/organization-workflow.js";

function input() {
  return {
    locale: "zh-CN",
    sessions: [
      {
        tool: "codex",
        id: "session-a",
        ref: "fsr_session_a",
        title: "测试修复",
        project: "ferry",
      },
    ],
  };
}

describe("organization workflow", () => {
  it("owns backbone, digest and proposal orchestration in the runtime", async () => {
    const calls: Array<{
      method: OrganizationEngineMethod;
      params: Record<string, unknown>;
    }> = [];
    const engine = {
      async invoke(
        method: OrganizationEngineMethod,
        params: Record<string, unknown>,
      ) {
        calls.push({ method, params });
        if (method === "session_backbone") {
          return {
            fingerprint: "fingerprint-a",
            pending: ["sha256:a"],
            pending_sources: [
              { hash: "sha256:a", text: "修复测试并完成验证。" },
            ],
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
    };
    let generatedInput: unknown;
    const result = await runOrganizationWorkflow(
      input(),
      "workflow-a",
      engine,
      async (value) => {
        generatedInput = value;
        return {
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
          clusters: [
            {
              id: "ferry",
              name: "Ferry",
              members: [{ tool: "codex", id: "session-a" }],
            },
          ],
        };
      },
    );

    expect(result).toEqual({ proposal_id: "proposal-a", status: "pending" });
    expect(generatedInput).toMatchObject({
      locale: "zh-CN",
      sessions: [
        {
          tool: "codex",
          id: "session-a",
          segments: [
            {
              hash: "sha256:a",
              text: "修复测试并完成验证。",
            },
          ],
        },
      ],
    });
    expect(calls.map((call) => call.method)).toEqual([
      "session_backbone",
      "organization_proposals_list",
      "session_summaries_set",
      "organization_digest_context",
      "organization_propose",
    ]);
    expect(calls.at(-1)?.params).toMatchObject({
      targets: [
        {
          tool: "codex",
          id: "session-a",
          fingerprint: "fingerprint-a",
          suggested: {
            cluster_id: "ferry",
            dead_candidate: false,
          },
        },
      ],
    });
  });

  it("returns an unchanged-fingerprint proposal without calling the model", async () => {
    let generated = false;
    const engine = {
      async invoke(method: OrganizationEngineMethod) {
        if (method === "session_backbone") {
          return {
            fingerprint: "fingerprint-a",
            pending: [],
            segments: [{ hash: "sha256:a", digest: "cached" }],
          };
        }
        return [
          {
            proposal_id: "cached",
            status: "pending",
            targets: [
              {
                tool: "codex",
                id: "session-a",
                fingerprint: "fingerprint-a",
              },
            ],
          },
        ];
      },
    };

    const result = await runOrganizationWorkflow(
      input(),
      "workflow-a",
      engine,
      async () => {
        generated = true;
        throw new Error("must not run");
      },
    );

    expect(result).toMatchObject({ proposal_id: "cached" });
    expect(generated).toBe(false);
  });

  it("rejects duplicate session identities before touching the engine", async () => {
    let invoked = false;
    await expect(
      runOrganizationWorkflow(
        { sessions: [input().sessions[0], input().sessions[0]] },
        "workflow-a",
        {
          async invoke() {
            invoked = true;
            return {};
          },
        },
        async () => ({ sessions: [], clusters: [] }),
      ),
    ).rejects.toThrow("sessions must be unique");
    expect(invoked).toBe(false);
  });
});
