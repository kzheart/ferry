import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { describe, expect, it, vi } from "vitest";
import { dispatch } from "../src/runtime/command-router.js";
import { AgentRuntime } from "../src/runtime/runtime.js";
import { PROTOCOL_VERSION, parseCommand } from "../src/server/messages.js";
import type { ProviderHost } from "../src/providers/provider-host.js";
import { createFerryTools, FERRY_TOOL_NAMES } from "../src/tools/catalog.js";
import { generateTitles } from "../src/titles/title-generator.js";
import {
  buildTitlePrompt,
  type TitleEvidence,
} from "../src/titles/title-prompt.js";
import {
  DEFAULT_TITLE_STYLE,
  normalizeTitleStyle,
  type TitleStyle,
} from "../src/titles/title-style.js";
import { createProtocolTestBackend } from "./test-backend.js";

const requireFromPiAi = createRequire(
  import.meta.resolve("@earendil-works/pi-ai"),
);
const { Check } = await import(
  pathToFileURL(requireFromPiAi.resolve("typebox/value")).href
);

function evidence(
  ref: string,
  overrides: Partial<TitleEvidence> = {},
): TitleEvidence {
  return {
    tool: "claude",
    ref,
    session_id: `sid-${ref}`,
    revision: 3,
    title: "旧标题",
    project: "/repo/ferry",
    message_count: 20,
    user_messages: ["把会话标题改成原生改名"],
    last_assistant_message: "已经改好了",
    files: ["src/titles/title-generator.ts"],
    ...overrides,
  };
}

function reply(entries: Array<Record<string, unknown>>) {
  return `\`\`\`json\n${JSON.stringify(entries)}\n\`\`\``;
}

function style(overrides: Partial<TitleStyle> = {}): TitleStyle {
  return { ...DEFAULT_TITLE_STYLE, ...overrides };
}

describe("标题风格归一化", () => {
  it("忽略未知字段并回落非法值", () => {
    const normalized = normalizeTitleStyle({
      preset: "nope",
      language: "jp",
      max_chars: 999,
      type_prefix: "yes",
      types: [{ emoji: "🔥", zh: "上线", en: "ship" }, { emoji: "" }],
      instructions: "全部用动词开头",
      examples: ["✨ 实现标题重置", 42],
      unknown_field: true,
    });
    expect(normalized).toMatchObject({
      preset: "ferry",
      language: "follow",
      max_chars: 60,
      type_prefix: true,
      types: [{ emoji: "🔥", zh: "上线", en: "ship" }],
      instructions: "全部用动词开头",
      examples: ["✨ 实现标题重置"],
    });
    expect(normalizeTitleStyle(undefined)).toEqual(DEFAULT_TITLE_STYLE);
  });
});

describe("标题提示词", () => {
  it("按硬约束→预设→用户指令→示例→证据→输出要求拼装", () => {
    const prompt = buildTitlePrompt(
      style({ instructions: "偏好口语", examples: ["✨ 实现标题重置"] }),
      [evidence("fsr_a")],
    );
    const order = [
      "硬约束",
      "预设「ferry」",
      "偏好口语",
      "示例标题",
      "会话证据",
      "输出要求",
    ].map((marker) => prompt.indexOf(marker));
    expect(order.every((index) => index >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(prompt).toContain("不超过 16");
    expect(prompt).toContain("✨  实现 / feat");
    expect(prompt).toContain("fsr_a");
    expect(prompt).toContain("把会话标题改成原生改名");
    expect(prompt).toContain("涉及文件: src/titles/title-generator.ts");
    expect(prompt).toContain('{"ref": "...", "type": "...", "title": "..."');
  });

  it("english 预设不给类型表,zh 语言写死中文", () => {
    const prompt = buildTitlePrompt(
      style({ preset: "english", language: "zh" }),
      [evidence("fsr_a")],
    );
    expect(prompt).toContain("type 一律填空字符串");
    expect(prompt).not.toContain("✨  实现 / feat");
    expect(prompt).toContain("一律用简体中文");
  });
});

describe("标题生成", () => {
  it("消息数不足的会话不进模型", async () => {
    const complete = vi.fn();
    const items = await generateTitles({
      sessions: [evidence("fsr_a", { message_count: 2 })],
      style: style(),
      complete,
    });
    expect(complete).not.toHaveBeenCalled();
    expect(items[0]).toMatchObject({
      ref: "fsr_a",
      skip: true,
      reason: "too_short",
      title: null,
      before: "旧标题",
      session_id: "sid-fsr_a",
      revision: 3,
    });
  });

  it("拼上类型前缀并带回会话标识", async () => {
    const complete = vi
      .fn()
      .mockResolvedValue(
        reply([{ ref: "fsr_a", type: "✨", title: "实现会话标题原生改名" }]),
      );
    const [item] = await generateTitles({
      sessions: [evidence("fsr_a")],
      style: style(),
      complete,
    });
    expect(item).toMatchObject({
      tool: "claude",
      ref: "fsr_a",
      title: "✨ 实现会话标题原生改名",
      type: "✨",
      skip: false,
      reason: null,
      before: "旧标题",
    });
  });

  it("剥掉首尾引号与模型自带的前缀,并按码点算长度", async () => {
    const complete = vi
      .fn()
      .mockResolvedValue(
        reply([{ ref: "fsr_a", type: "fix", title: '"🐛 修复\n多行"' }]),
      );
    const [item] = await generateTitles({
      sessions: [evidence("fsr_a")],
      style: style({ max_chars: 6 }),
      complete,
    });
    expect(item?.title).toBe("🐛 修复");
    expect(complete).toHaveBeenCalledTimes(1);
  });

  it("超长先重试一次,重试仍超长才跳过", async () => {
    const long = "实".repeat(20);
    const complete = vi
      .fn()
      .mockResolvedValueOnce(reply([{ ref: "fsr_a", type: "✨", title: long }]))
      .mockResolvedValueOnce(
        reply([{ ref: "fsr_a", type: "✨", title: "实现改名" }]),
      );
    const [ok] = await generateTitles({
      sessions: [evidence("fsr_a")],
      style: style(),
      complete,
    });
    expect(ok).toMatchObject({ title: "✨ 实现改名", skip: false });
    expect(complete.mock.calls[1]?.[0]).toContain("超出了长度上限");

    const stubborn = vi
      .fn()
      .mockResolvedValue(reply([{ ref: "fsr_a", type: "✨", title: long }]));
    const [failed] = await generateTitles({
      sessions: [evidence("fsr_a")],
      style: style(),
      complete: stubborn,
    });
    expect(failed).toMatchObject({
      skip: true,
      reason: "too_long",
      title: null,
    });
    expect(stubborn).toHaveBeenCalledTimes(2);
  });

  it("同项目内重名的第二个重试一次,仍重名就跳过", async () => {
    const complete = vi.fn().mockResolvedValue(
      reply([
        { ref: "fsr_a", type: "✨", title: "实现改名" },
        { ref: "fsr_b", type: "✨", title: "实现改名" },
      ]),
    );
    const items = await generateTitles({
      sessions: [evidence("fsr_a"), evidence("fsr_b")],
      style: style(),
      complete,
    });
    expect(items[0]).toMatchObject({ title: "✨ 实现改名", skip: false });
    expect(items[1]).toMatchObject({ skip: true, reason: "duplicate" });
    expect(complete).toHaveBeenCalledTimes(2);
    expect(complete.mock.calls[1]?.[0]).toContain("重复");
  });

  it("跨项目允许重名", async () => {
    const complete = vi.fn().mockResolvedValue(
      reply([
        { ref: "fsr_a", type: "✨", title: "实现改名" },
        { ref: "fsr_b", type: "✨", title: "实现改名" },
      ]),
    );
    const items = await generateTitles({
      sessions: [
        evidence("fsr_a"),
        evidence("fsr_b", { project: "/repo/other" }),
      ],
      style: style(),
      complete,
    });
    expect(items.map((item) => item.title)).toEqual([
      "✨ 实现改名",
      "✨ 实现改名",
    ]);
    expect(complete).toHaveBeenCalledTimes(1);
  });

  it("解析失败与类型非法都重试一次,再失败标 invalid", async () => {
    const complete = vi.fn().mockResolvedValue("模型今天不想说话");
    const [item] = await generateTitles({
      sessions: [evidence("fsr_a")],
      style: style(),
      complete,
    });
    expect(item).toMatchObject({ skip: true, reason: "invalid" });
    expect(complete).toHaveBeenCalledTimes(2);

    const badType = vi
      .fn()
      .mockResolvedValue(
        reply([{ ref: "fsr_a", type: "🍕", title: "实现改名" }]),
      );
    const [typed] = await generateTitles({
      sessions: [evidence("fsr_a")],
      style: style(),
      complete: badType,
    });
    expect(typed).toMatchObject({ skip: true, reason: "invalid" });
  });

  it("模型自己要求跳过时原样带回原因,不再重试", async () => {
    const complete = vi.fn().mockResolvedValue(
      reply([
        {
          ref: "fsr_a",
          type: "✨",
          title: "",
          skip: true,
          reason: "内容太杂",
        },
      ]),
    );
    const [item] = await generateTitles({
      sessions: [evidence("fsr_a")],
      style: style(),
      complete,
    });
    expect(item).toMatchObject({ skip: true, reason: "内容太杂" });
    expect(complete).toHaveBeenCalledTimes(1);
  });

  it("每批最多 10 个会话", async () => {
    const sessions = Array.from({ length: 12 }, (_, index) =>
      evidence(`fsr_${index}`),
    );
    const complete = vi.fn(async (prompt: string) =>
      reply(
        [...prompt.matchAll(/ref: (fsr_\d+)/g)].map((match, index) => ({
          ref: match[1],
          type: "✨",
          title: `实现改名${index}${prompt.length % 7}`,
        })),
      ),
    );
    const items = await generateTitles({
      sessions,
      style: style(),
      complete,
    });
    expect(complete).toHaveBeenCalledTimes(2);
    expect(items.filter((item) => item.skip)).toHaveLength(0);
  });

  it("english 预设不加前缀,bracket 预设用类型名", async () => {
    const english = await generateTitles({
      sessions: [evidence("fsr_a")],
      style: style({ preset: "english", max_chars: 40 }),
      complete: async () =>
        reply([{ ref: "fsr_a", type: "", title: "native session rename" }]),
    });
    expect(english[0]).toMatchObject({
      title: "native session rename",
      type: null,
    });

    const bracket = await generateTitles({
      sessions: [evidence("fsr_a")],
      style: style({ preset: "bracket" }),
      complete: async () =>
        reply([{ ref: "fsr_a", type: "✨", title: "实现原生改名" }]),
    });
    expect(bracket[0]).toMatchObject({
      title: "[实现] 实现原生改名",
      type: "实现",
    });
  });
});

function fakeHost(complete: (prompt: string) => Promise<string>) {
  return {
    async defaultModel() {
      return { provider: "openai", model: "gpt-test" };
    },
    async titleCompleter() {
      return {
        selection: { provider: "openai", model: "gpt-test" },
        complete,
      };
    },
  } as unknown as ProviderHost;
}

function command(params: Record<string, unknown>) {
  return parseCommand({
    protocol: PROTOCOL_VERSION,
    id: "1",
    method: "title.generate",
    params,
  });
}

describe("title.generate 命令", () => {
  it("返回条目、所用模型与跳过数", async () => {
    const runtime = await AgentRuntime.create({
      backendFactory: createProtocolTestBackend,
      providerHost: fakeHost(async () =>
        reply([{ ref: "fsr_a", type: "✨", title: "实现原生改名" }]),
      ),
    });
    const response = await dispatch(
      runtime,
      command({
        sessions: [
          { tool: "claude", ref: "fsr_a", message_count: 9, title: "旧标题" },
          { tool: "codex", ref: "fsr_b", message_count: 1 },
        ],
        style: { max_chars: 20, unknown: true },
      }),
    );
    expect(response).toMatchObject({
      ok: true,
      result: {
        items: [
          {
            tool: "claude",
            ref: "fsr_a",
            title: "✨ 实现原生改名",
            type: "✨",
            skip: false,
            before: "旧标题",
          },
          { tool: "codex", ref: "fsr_b", skip: true, reason: "too_short" },
        ],
        model: { provider_id: "openai", model: "gpt-test" },
        skipped: 1,
      },
    });
  });

  it("会话数越界报 invalid_params", async () => {
    const runtime = await AgentRuntime.create({
      backendFactory: createProtocolTestBackend,
      providerHost: fakeHost(async () => reply([])),
    });
    const response = await dispatch(runtime, command({ sessions: [] }));
    expect(response).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
  });

  it("没有可用 provider 时报 provider_unavailable", async () => {
    const runtime = await AgentRuntime.create({
      backendFactory: createProtocolTestBackend,
    });
    const response = await dispatch(
      runtime,
      command({ sessions: [{ tool: "claude", ref: "fsr_a" }] }),
    );
    expect(response).toMatchObject({
      ok: false,
      error: { code: "provider_unavailable", retryable: true },
    });
  });
});

describe("session_title_evidence 工具", () => {
  const tools = createFerryTools(
    {
      async invoke() {
        return {};
      },
    },
    () => ({ sessionId: "session", runId: "run" }),
  );
  const tool = tools.find((item) => item.name === "session_title_evidence")!;

  it("在工具清单里,只读并行", () => {
    expect(FERRY_TOOL_NAMES).toContain("session_title_evidence");
    expect(tool.executionMode).toBe("parallel");
    expect(tool.description).toContain("Read-only");
    expect(tool.description).toContain("session_edit title");
    expect(tool.description).toContain("manual");
    expect(tool.description).toContain("message_count < 3");
  });

  it("schema 限定 1..50 个 {tool, ref}", () => {
    const ref = "fsr_abcdefgh";
    expect(
      Check(tool.parameters, { sessions: [{ tool: "claude", ref }] }),
    ).toBe(true);
    expect(Check(tool.parameters, { sessions: [] })).toBe(false);
    expect(
      Check(tool.parameters, {
        sessions: Array.from({ length: 51 }, () => ({ tool: "claude", ref })),
      }),
    ).toBe(false);
    expect(
      Check(tool.parameters, { sessions: [{ tool: "claude", ref, extra: 1 }] }),
    ).toBe(false);
    expect(Check(tool.parameters, { sessions: [{ ref }] })).toBe(false);
  });

  it("原样透传给宿主", async () => {
    const invoke = vi.fn(async () => ({ sessions: [], style: {} }));
    const [routed] = createFerryTools(
      { invoke },
      () => ({ sessionId: "session", runId: "run" }),
      ["session_title_evidence"],
    );
    const args = { sessions: [{ tool: "claude", ref: "fsr_abcdefgh" }] };
    await routed!.execute("call-1", args, undefined as never, undefined);
    expect(invoke).toHaveBeenCalledWith(
      "session_title_evidence",
      args,
      expect.objectContaining({ sessionId: "session", toolCallId: "call-1" }),
    );
  });
});
