import { Readable } from "node:stream";
import { describe, expect, it } from "vitest";
import { readJsonLines } from "../src/protocol/jsonl.js";

async function collect(chunks: string[], maxBytes?: number) {
  const result: string[] = [];
  for await (const line of readJsonLines(Readable.from(chunks), maxBytes)) {
    result.push(line);
  }
  return result;
}

describe("strict LF JSONL framing", () => {
  it("does not split valid JSON strings containing Unicode separators", async () => {
    const first = JSON.stringify({ text: "left\u2028middle\u2029right" });
    await expect(collect([`${first}\n`, '{"ok":true}\r\n'])).resolves.toEqual([
      first,
      '{"ok":true}',
    ]);
  });

  it("handles records split across arbitrary byte chunks", async () => {
    await expect(collect(['{"a"', ':1}\n{"b":', "2}\n"])).resolves.toEqual([
      '{"a":1}',
      '{"b":2}',
    ]);
  });

  it("rejects oversized and unterminated records", async () => {
    await expect(collect(["12345"], 4)).rejects.toThrow("size limit");
    await expect(collect(["{}"])).rejects.toThrow("without LF");
  });
});
