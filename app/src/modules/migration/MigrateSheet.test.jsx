// 迁移面板上与「续聊到」有关的唯一可见行为:迁移在「影响」步倒在了前置条件上时,
// 给出一条零写入的退路——复制指令,不切换面板步骤。
import { beforeEach, test, vi } from "vitest";
import assert from "node:assert/strict";
import { act, fireEvent, render, screen } from "@testing-library/react";

let planFailure = null;

vi.mock("../../platform/desktop/client.js", () => ({
  engine: async () => ({}),
  openTerminal: async () => {},
  revealPath: async () => {},
  writeClipboardText: async () => {},
  integrationStatus: async () => ({ skills: [] }),
}));

vi.mock("../operations/public.js", () => ({
  operations: {
    plan: async () => { throw planFailure || new Error("boom"); },
    apply: async () => ({ result: {} }),
  },
}));

const { default: MigrateSheet } = await import("./MigrateSheet.jsx");

const meta = { tool: "claude", id: "native-1", title: "会话", ref: "fsr_x" };
const env = {
  claude: { installed: true },
  codex: { installed: true },
  cursor: { installed: true },
  opencode: { installed: false },
};

const sheet = (props = {}) => render(
  <MigrateSheet meta={meta} scope={null} env={env} onClose={() => {}} {...props} />,
);

beforeEach(() => {
  planFailure = null;
});

test("目标列表是迁移那一份:源工具自己不在里面", async () => {
  await act(async () => { sheet(); });
  assert.equal(screen.queryByText("Claude Code"), null);
  assert.ok(screen.getByText("Codex CLI"));
});

test("源存储被占用时给出「改为续聊」,点了只复制、不换步骤", async () => {
  let picked = 0;
  planFailure = Object.assign(new Error("busy"), {
    code: "session.store_unavailable",
    params: {},
  });
  await act(async () => {
    sheet({ onResumeElsewhere: () => { picked += 1; } });
  });
  await act(async () => { fireEvent.click(screen.getByText("migration:sheet.next")); });

  const fallback = screen.getByText(/migration:resume.fallback/);
  await act(async () => { fireEvent.click(fallback); });
  assert.equal(picked, 1);
  // 面板留在「影响」步:退路是复制一条指令,不是换一套流程
  assert.ok(screen.getByText(/migration:preview.failed/));
});

test("与续聊无关的失败不给退路按钮", async () => {
  planFailure = Object.assign(new Error("gone"), {
    code: "session.not_found",
    params: {},
  });
  await act(async () => { sheet(); });
  await act(async () => { fireEvent.click(screen.getByText("migration:sheet.next")); });

  assert.equal(screen.queryByText(/migration:resume.fallback/), null);
});
