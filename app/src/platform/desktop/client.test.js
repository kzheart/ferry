import { expect, test, vi } from "vitest";

// Tauri v2 的命令参数默认按 camelCase 查键,且查不到不会回退到 snake_case——
// 参数名写成 request_id 会让每一次应答都以 "missing required key" 失败。
const calls = [];
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (command, args) => {
    calls.push({ command, args });
    return undefined;
  },
}));

const { choiceRespond, operationApply, shellApply } =
  await import("./client.js");

test("choiceRespond 用 camelCase 参数名调用 choice_respond", async () => {
  const answer = { answered: true, selected: ["A"], custom_text: "" };
  await choiceRespond("session-1", "req-1", answer);

  expect(calls.at(-1)).toEqual({
    command: "choice_respond",
    args: { sessionId: "session-1", requestId: "req-1", answer },
  });
  // snake_case 键一个都不能漏出去
  expect(Object.keys(calls.at(-1).args).some(key => key.includes("_")))
    .toBe(false);
});

test("其余命令参数名同样是 camelCase", async () => {
  await shellApply("shl_1");
  expect(calls.at(-1)).toEqual({
    command: "bash_apply", args: { planId: "shl_1" },
  });

  await operationApply("op_1").catch(() => {});
  expect(calls.at(-1).args).toEqual({ planId: "op_1" });
});
