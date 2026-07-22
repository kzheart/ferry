import {
  createAssistantMessageEventStream,
  type AssistantMessage,
  type Context,
  type Model,
  type SimpleStreamOptions,
  type StreamFunction,
  type ToolCall,
} from "@earendil-works/pi-ai";
import type { AgentBackend } from "../src/runtime.js";

const usage = {
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  totalTokens: 0,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};

const model: Model<string> = {
  id: "protocol-test-driver",
  name: "Protocol test driver",
  api: "protocol-test",
  provider: "protocol-test",
  baseUrl: "http://127.0.0.1",
  reasoning: false,
  input: ["text"],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  contextWindow: 16_384,
  maxTokens: 4_096,
};

function message(
  content: AssistantMessage["content"],
  stopReason: AssistantMessage["stopReason"],
): AssistantMessage {
  return {
    role: "assistant",
    content,
    api: model.api,
    provider: model.provider,
    model: model.id,
    usage,
    stopReason,
    timestamp: Date.now(),
  };
}

function lastUserText(context: Context) {
  const last = [...context.messages]
    .reverse()
    .find((item) => item.role === "user");
  if (!last || last.role !== "user") return "";
  return typeof last.content === "string"
    ? last.content
    : last.content
        .filter((part) => part.type === "text")
        .map((part) => part.text)
        .join("\n");
}

async function delay(milliseconds: number, signal?: AbortSignal) {
  await new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, milliseconds);
    signal?.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
  });
}

const streamFn: StreamFunction = (
  _model,
  context,
  options?: SimpleStreamOptions,
) => {
  const stream = createAssistantMessageEventStream();
  void (async () => {
    const last = context.messages.at(-1);
    if (last?.role === "toolResult") {
      const value =
        last.content[0]?.type === "text" ? last.content[0].text : "ok";
      emitText(stream, `Tool result: ${value}`);
      return;
    }
    const prompt = lastUserText(context);
    if (prompt.startsWith("tool:list_capabilities")) {
      const call: ToolCall = {
        type: "toolCall",
        id: "tool-call-1",
        name: "ferry_list_capabilities",
        arguments: {},
      };
      const partial = message([], "toolUse");
      stream.push({ type: "start", partial });
      const complete = message([call], "toolUse");
      stream.push({ type: "toolcall_start", contentIndex: 0, partial });
      stream.push({
        type: "toolcall_end",
        contentIndex: 0,
        toolCall: call,
        partial: complete,
      });
      stream.push({ type: "done", reason: "toolUse", message: complete });
      return;
    }
    if (prompt.startsWith("slow:")) await delay(40, options?.signal);
    if (options?.signal?.aborted) {
      const aborted = {
        ...message([], "aborted"),
        errorMessage: "Request was aborted",
      };
      stream.push({ type: "error", reason: "aborted", error: aborted });
      return;
    }
    emitText(stream, `Echo: ${prompt}`);
  })();
  return stream;
};

function emitText(
  stream: ReturnType<typeof createAssistantMessageEventStream>,
  text: string,
) {
  const partial = message([], "stop");
  stream.push({ type: "start", partial });
  stream.push({ type: "text_start", contentIndex: 0, partial });
  const complete = message([{ type: "text", text }], "stop");
  stream.push({
    type: "text_delta",
    contentIndex: 0,
    delta: text,
    partial: complete,
  });
  stream.push({
    type: "text_end",
    contentIndex: 0,
    content: text,
    partial: complete,
  });
  stream.push({ type: "done", reason: "stop", message: complete });
}

export function createProtocolTestBackend(): AgentBackend {
  return {
    model,
    streamFn,
    provider: model.provider,
    modelId: model.id,
    credentialAvailable: () => true,
  };
}
