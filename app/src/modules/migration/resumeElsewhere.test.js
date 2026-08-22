// 「续聊到」只产出一条给用户粘贴的指令,所以能钉住的就是这条指令的形状,
// 以及复制之后的三档反馈(装了 / 没装 / 读不到状态)。
import { beforeEach, test, vi } from "vitest";
import assert from "node:assert/strict";

let clipboard = null;
let status = null;
let statusFails = false;

vi.mock("../../platform/desktop/client.js", () => ({
  writeClipboardText: async text => { clipboard = text; },
  integrationStatus: async () => {
    if (statusFails) throw new Error("no host");
    return status;
  },
}));

const {
  buildResumeCommand,
  canFallBackToResume,
  copyResumeCommand,
} = await import("./resumeElsewhere.js");

const t = (key, params) => (params ? `${key}:${JSON.stringify(params)}` : key);

beforeEach(() => {
  clipboard = null;
  statusFails = false;
  status = { skills: [{ id: "shared", installed: true }] };
});

test("指令形状固定为 /ferry-resume <tool> <原生 id>,与目标无关", () => {
  assert.equal(
    buildResumeCommand({ tool: "codex", sessionId: "01a0-28" }),
    "/ferry-resume codex 01a0-28",
  );
});

test("缺 tool 或 sessionId 一律给空串,绝不复制半条指令", () => {
  assert.equal(buildResumeCommand({ tool: "codex" }), "");
  assert.equal(buildResumeCommand({ sessionId: "x" }), "");
  assert.equal(buildResumeCommand(), "");
});

test("只有存储被占用或目标不支持迁入这两种失败才给续聊退路", () => {
  assert.equal(canFallBackToResume({ code: "session.store_unavailable" }), true);
  assert.equal(canFallBackToResume({
    code: "agent.request_invalid", params: { capability: "migration-target" },
  }), true);
  assert.equal(canFallBackToResume({ code: "session.not_found" }), false);
  assert.equal(canFallBackToResume(null), false);
});

test("共享技能目录装了 skill 时打勾,不点名任何 agent", async () => {
  let toast = null;
  await copyResumeCommand({
    tool: "claude", sessionId: "sid-1",
    t, setToast: value => { toast = value; },
  });
  assert.equal(clipboard, "/ferry-resume claude sid-1");
  assert.equal(toast.kind, "ok");
  assert.equal(toast.title, "app:toast.resumeCopied");
  assert.equal(toast.desc, "app:toast.resumeCopiedDesc");
  assert.equal(toast.action, undefined);
});

test("skill 没装时不打勾,改为警告并给去安装入口", async () => {
  status = { skills: [{ id: "shared", installed: false }] };
  let toast = null;
  let opened = null;
  await copyResumeCommand({
    tool: "claude", sessionId: "sid-1",
    t, setToast: value => { toast = value; }, openConfig: s => { opened = s; },
  });
  assert.equal(clipboard, "/ferry-resume claude sid-1");
  assert.equal(toast.kind, "warn");
  assert.equal(toast.title, "app:toast.resumeCopiedNoSkill");
  toast.action.onClick();
  assert.equal(opened, "integration");
});

test("读不到集成状态时给中性说明:不确定不等于没装", async () => {
  statusFails = true;
  let toast = null;
  await copyResumeCommand({
    tool: "claude", sessionId: "sid-1",
    t, setToast: value => { toast = value; },
  });
  assert.equal(toast.kind, "ok");
  assert.equal(toast.desc, "app:toast.resumeCopiedDesc");
  assert.equal(toast.action, undefined);
});
