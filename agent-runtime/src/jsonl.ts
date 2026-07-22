import type { Readable } from "node:stream";

export const MAX_JSONL_RECORD_BYTES = 2 * 1024 * 1024;

export async function* readJsonLines(
  input: Readable,
  maxBytes = MAX_JSONL_RECORD_BYTES,
): AsyncGenerator<string> {
  let pending = Buffer.alloc(0);
  for await (const chunk of input) {
    const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk as string);
    pending = Buffer.concat([pending, bytes]);
    if (pending.length > maxBytes && pending.indexOf(0x0a) === -1) {
      throw new Error("JSONL record exceeds size limit");
    }
    let newline = pending.indexOf(0x0a);
    while (newline !== -1) {
      if (newline > maxBytes)
        throw new Error("JSONL record exceeds size limit");
      let record = pending.subarray(0, newline);
      pending = pending.subarray(newline + 1);
      if (record.at(-1) === 0x0d) record = record.subarray(0, -1);
      yield record.toString("utf8");
      newline = pending.indexOf(0x0a);
    }
  }
  if (pending.length > 0)
    throw new Error("JSONL input ended without LF delimiter");
}
