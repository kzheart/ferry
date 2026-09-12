import { describe, expect, it, vi } from "vitest";
import {
  estimateTokens,
  type AgentMessage,
} from "@earendil-works/pi-agent-core";
import type {
  AssistantMessage,
  Context,
  Models,
  Usage,
} from "@earendil-works/pi-ai";
import { compactContext } from "../src/sessions/context.js";
import { createProtocolTestBackend } from "./test-backend.js";

const model = { ...createProtocolTestBackend().model, contextWindow: 8_192 };
const usage = (tokens = 0): Usage => ({
  input: tokens,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  totalTokens: tokens,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
});
const user = (text: string): AgentMessage => ({
  role: "user",
  content: text,
  timestamp: 1,
});
const assistant = (text: string, tokens = 0): AssistantMessage => ({
  role: "assistant",
  content: [{ type: "text", text }],
  api: model.api,
  provider: model.provider,
  model: model.id,
  usage: usage(tokens),
  stopReason: "stop",
  timestamp: 2,
});
function modelsMock(
  complete = async () => assistant("SUMMARY: retain the goal and decisions"),
) {
  const completeSimple = vi.fn(async (_model: unknown, _context: Context) =>
    complete(),
  );
  return { models: { completeSimple } as unknown as Models, completeSimple };
}
function longHistory(): AgentMessage[] {
  return [
    user("Original goal: " + "a".repeat(15_000)),
    assistant("b".repeat(15_000)),
    user("Recent request"),
    assistant("Recent answer"),
    user("Continue"),
  ];
}
function textOf(messages: AgentMessage[]) {
  return JSON.stringify(messages);
}

describe("Ferry 模型上下文压缩", () => {
  it("超预算压缩保留工具调用配对，且不修改原始历史", async () => {
    const toolCall: AssistantMessage = {
      ...assistant(""),
      stopReason: "toolUse",
      content: [
        {
          type: "toolCall",
          id: "call-1",
          name: "session_read",
          arguments: { ref: "fsr_test" },
        },
      ],
    };
    const history = [
      ...longHistory().slice(0, 2),
      user("Inspect current session"),
      toolCall,
      {
        role: "toolResult",
        toolCallId: "call-1",
        toolName: "session_read",
        content: [{ type: "text", text: "Result " + "r".repeat(2_000) }],
        isError: false,
        timestamp: 3,
      } as AgentMessage,
    ];
    const original = structuredClone(history);
    const { models, completeSimple } = modelsMock();
    const result = await compactContext(
      history,
      undefined,
      models,
      model,
      "System prompt",
    );
    expect(completeSimple).toHaveBeenCalled();
    expect(result.checkpoint?.messageCount).toBe(history.length);
    expect(textOf(result.messages)).toContain("SUMMARY");
    expect(result.messages).toContainEqual(toolCall);
    const resultIndex = result.messages.findIndex(
      (message) => message.role === "toolResult",
    );
    expect(resultIndex).toBeGreaterThan(0);
    expect(result.messages[resultIndex - 1]).toEqual(toolCall);
    expect(
      result.messages.reduce(
        (total, message) => total + estimateTokens(message),
        0,
      ),
    ).toBeLessThan(6_144);
    expect(history).toEqual(original);
  });

  it("恢复 checkpoint 后追加全部新消息，不重复旧历史", async () => {
    const history = longHistory();
    const { models, completeSimple } = modelsMock();
    const first = await compactContext(history, undefined, models, model, "");
    expect(first.checkpoint).toBeDefined();
    const checkpoint = structuredClone(first.checkpoint!);
    const additions = [assistant("After restore"), user("New instruction")];
    const result = await compactContext(
      [...history, ...additions],
      checkpoint,
      models,
      model,
      "",
    );
    expect(result.messages).toEqual([...checkpoint.messages, ...additions]);
    expect(completeSimple).toHaveBeenCalledTimes(1);
    expect(textOf(result.messages)).not.toContain("Original goal:");
    expect(checkpoint).toEqual(first.checkpoint);
  });

  it("再次压缩时上次摘要仍进入模型输入", async () => {
    const { models, completeSimple } = modelsMock();
    const history = longHistory();
    const first = await compactContext(history, undefined, models, model, "");
    const additions = [
      user("Next phase " + "c".repeat(14_000)),
      assistant("Progress"),
      user("Another phase " + "d".repeat(14_000)),
      assistant("More progress"),
      user("Continue"),
    ];
    const second = await compactContext(
      [...history, ...additions],
      first.checkpoint,
      models,
      model,
      "",
    );
    expect(second.checkpoint?.messageCount).toBe(
      history.length + additions.length,
    );
    expect(JSON.stringify(completeSimple.mock.calls.slice(1))).toContain(
      "SUMMARY: retain the goal and decisions",
    );
  });

  it("压缩被取消时不产生 checkpoint，也不改动历史", async () => {
    const controller = new AbortController();
    const { models } = modelsMock(async () => {
      controller.abort();
      return assistant("Discard this summary");
    });
    const history = longHistory();
    const original = structuredClone(history);
    await expect(
      compactContext(history, undefined, models, model, "", controller.signal),
    ).rejects.toThrow();
    expect(history).toEqual(original);
  });

  it("provider usage 已包含系统提示时不重复计算系统 token", async () => {
    const { models, completeSimple } = modelsMock();
    const result = await compactContext(
      [user("hi"), assistant("hello", 5_000), user("next")],
      undefined,
      models,
      model,
      "s".repeat(6_000),
    );
    expect(completeSimple).not.toHaveBeenCalled();
    expect(result.checkpoint).toBeUndefined();
  });

  it("压缩后保留的旧 usage 不触发下一次无意义压缩", async () => {
    const history = [
      ...longHistory().slice(0, 3),
      assistant("Retained answer", 7_000),
      user("Continue"),
    ];
    const { models, completeSimple } = modelsMock();
    const first = await compactContext(history, undefined, models, model, "");
    expect(first.checkpoint).toBeDefined();
    const before = completeSimple.mock.calls.length;
    const second = await compactContext(
      history,
      first.checkpoint,
      models,
      model,
      "",
    );
    expect(second.messages).toEqual(first.messages);
    expect(second.checkpoint).toBeUndefined();
    expect(completeSimple).toHaveBeenCalledTimes(before);
  });

  it("checkpoint 后的新 provider usage 仍能触发下一次压缩", async () => {
    const history = longHistory();
    const { models, completeSimple } = modelsMock();
    const first = await compactContext(history, undefined, models, model, "");
    const before = completeSimple.mock.calls.length;
    const additions = [
      assistant("A newly measured response", 7_000),
      user("Next"),
    ];
    const result = await compactContext(
      [...history, ...additions],
      first.checkpoint,
      models,
      model,
      "",
    );
    expect(result.checkpoint?.messageCount).toBe(
      history.length + additions.length,
    );
    expect(completeSimple.mock.calls.length).toBeGreaterThan(before);
  });

  it("模型返回超大摘要时不生成无法使用的 checkpoint", async () => {
    const { models } = modelsMock(async () => assistant("s".repeat(40_000)));
    await expect(
      compactContext(longHistory(), undefined, models, model, ""),
    ).rejects.toThrow("Compacted context exceeds");
  });

  it("当前请求本身超过窗口时明确拒绝，而非返回仍然超限的上下文", async () => {
    const { models } = modelsMock();
    await expect(
      compactContext([user("x".repeat(40_000))], undefined, models, model, ""),
    ).rejects.toThrow(/context|window|budget/i);
  });
});
