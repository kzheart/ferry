import { describe, expect, it } from "vitest";
import {
  organizerPrompt,
  parseOrganizerInput,
  validateOrganizerResult,
} from "../src/organizing/organizer.js";
import { ProviderHost } from "../src/providers/provider-host.js";

const input = parseOrganizerInput({
  locale: "zh-CN",
  sessions: [
    {
      tool: "codex",
      id: "s1",
      segments: [
        {
          hash: "sha256:a",
          text: "用户要求修复测试，助手完成修复并验证通过。",
        },
      ],
    },
    {
      tool: "claude",
      id: "s2",
      segments: [{ hash: "sha256:b", text: "讨论同一项目的发布流程。" }],
    },
  ],
});

describe("organizer contract", () => {
  it("builds a bounded prompt with source hashes", () => {
    const prompt = organizerPrompt(input);
    expect(prompt).toContain("sha256:a");
    expect(prompt).toContain("zh-CN");
    expect(prompt).toContain("supported only by that segment");
  });

  it("validates digests and cross-agent clusters against the input", () => {
    const result = validateOrganizerResult(
      JSON.stringify({
        sessions: [
          {
            tool: "codex",
            id: "s1",
            digests: { "sha256:a": "完成测试修复并验证。" },
            title: "修复测试",
            summary: "修复并验证测试。",
            tags: ["测试"],
            dead: false,
          },
          {
            tool: "claude",
            id: "s2",
            digests: { "sha256:b": "讨论发布流程。" },
            title: "发布流程",
            summary: "讨论项目发布。",
            tags: ["发布"],
            dead: false,
          },
        ],
        clusters: [
          {
            id: "project",
            name: "项目",
            members: [
              { tool: "codex", id: "s1" },
              { tool: "claude", id: "s2" },
            ],
          },
        ],
      }),
      input,
    );
    expect(result.sessions[0]?.digests["sha256:a"]).toBe(
      "完成测试修复并验证。",
    );
    expect(result.clusters[0]?.members).toHaveLength(2);
  });

  it("rejects hallucinated hashes and omitted sessions", () => {
    expect(() =>
      validateOrganizerResult(
        JSON.stringify({
          sessions: [
            {
              tool: "codex",
              id: "s1",
              digests: { "sha256:wrong": "x" },
              title: "x",
              summary: "x",
              tags: [],
              dead: false,
            },
          ],
          clusters: [],
        }),
        input,
      ),
    ).toThrow(/digest|omitted/);
  });

  it("uses ProviderHost completion and validates its structured response", async () => {
    const host = Object.create(ProviderHost.prototype) as ProviderHost & {
      models: { completeSimple: (...args: unknown[]) => Promise<unknown> };
    };
    Object.assign(host, {
      defaultModel: async () => ({ provider: "test", model: "m" }),
      model: () => ({ id: "m", provider: "test" }),
      isConfigured: async () => true,
      models: {
        completeSimple: async () => ({
          role: "assistant",
          content: [
            {
              type: "text",
              text: JSON.stringify({
                sessions: [
                  {
                    tool: "codex",
                    id: "s1",
                    digests: { "sha256:a": "完成测试修复并验证。" },
                    title: "修复测试",
                    summary: "修复并验证测试。",
                    tags: ["测试"],
                    dead: false,
                  },
                  {
                    tool: "claude",
                    id: "s2",
                    digests: { "sha256:b": "讨论发布流程。" },
                    title: "发布流程",
                    summary: "讨论项目发布。",
                    tags: ["发布"],
                    dead: false,
                  },
                ],
                clusters: [],
              }),
            },
          ],
          stopReason: "stop",
        }),
      },
    });
    const result = await host.organize(input);
    expect(result.sessions).toHaveLength(2);
  });
});
