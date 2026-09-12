import {
  compact,
  convertToLlm,
  createCompactionSummaryMessage,
  estimateContextTokens,
  estimateTokens,
  prepareCompaction,
  shouldCompact,
  type AgentMessage,
  type SessionTreeEntry,
} from "@earendil-works/pi-agent-core";
import type { Model, Models } from "@earendil-works/pi-ai";

/** A model-context checkpoint, independent of Ferry's complete conversation history. */
export interface ContextCheckpoint {
  messageCount: number;
  messages: AgentMessage[];
}

export function contextReserveTokens(model: Model<string>) {
  return Math.min(16_384, Math.floor(model.contextWindow / 4));
}

export async function compactContext(
  history: AgentMessage[],
  checkpoint: ContextCheckpoint | undefined,
  models: Models,
  model: Model<string>,
  systemPrompt: string,
  signal?: AbortSignal,
): Promise<{ messages: AgentMessage[]; checkpoint?: ContextCheckpoint }> {
  const activeCheckpoint =
    checkpoint && checkpoint.messageCount <= history.length
      ? checkpoint
      : undefined;
  const messages = activeCheckpoint
    ? [
        ...activeCheckpoint.messages,
        ...history.slice(activeCheckpoint.messageCount),
      ]
    : history;
  const settings = {
    enabled: true,
    reserveTokens: contextReserveTokens(model),
    keepRecentTokens: Math.min(20_000, Math.floor(model.contextWindow / 4)),
  };
  const systemTokens = Math.ceil(systemPrompt.length / 3);
  const estimate = estimateContextTokens(messages);
  // Provider usage already includes the system prompt. Retained pre-compaction
  // usage describes the old context, so only newer assistant usage is authoritative.
  const hasCurrentUsage =
    estimate.lastUsageIndex !== null &&
    (!activeCheckpoint ||
      estimate.lastUsageIndex >= activeCheckpoint.messages.length);
  const estimateMessages = (items: AgentMessage[]) =>
    items.reduce((total, message) => total + estimateTokens(message), 0) +
    systemTokens;
  const tokens = hasCurrentUsage ? estimate.tokens : estimateMessages(messages);
  if (!shouldCompact(tokens, model.contextWindow, settings))
    return { messages };
  // Only the pure compaction helpers consume these transient entries; Ferry owns persistence.
  const entries: SessionTreeEntry[] = messages.map((message, index) => ({
    type: "message",
    id: String(index),
    parentId: index ? String(index - 1) : null,
    timestamp: new Date(message.timestamp).toISOString(),
    message,
  }));
  const prepared = prepareCompaction(entries, settings);
  if (!prepared.ok) throw prepared.error;
  if (!prepared.value) return { messages };
  const contextBudget = model.contextWindow - settings.reserveTokens;
  if (estimateMessages(prepared.value.retainedTail) >= contextBudget) {
    throw new Error(
      "Recent messages exceed the model context budget; shorten the current request or tool output",
    );
  }
  const result = await compact(
    prepared.value,
    models,
    model,
    undefined,
    signal,
    "off",
    {
      enabled: true,
      maxRetries: 2,
      baseDelayMs: 1_000,
    },
  );
  if (!result.ok) throw result.error;
  signal?.throwIfAborted();
  const compressed: AgentMessage[] = convertToLlm([
    createCompactionSummaryMessage(
      result.value.summary,
      tokens,
      new Date().toISOString(),
    ),
    ...(result.value.retainedTail ?? prepared.value.retainedTail),
  ]);
  if (estimateMessages(compressed) > contextBudget) {
    throw new Error("Compacted context exceeds the model context budget");
  }
  return {
    messages: compressed,
    checkpoint: { messageCount: history.length, messages: compressed },
  };
}
