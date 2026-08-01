import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { ApprovalCard } from "./AgentApprovalCard.jsx";

test("cleanup approval card renders frozen totals and excluded entries", () => {
  render(<ApprovalCard
    item={{
      status: "pending",
      operation: {
        plan_id: "op_cleanup_fixture",
        kind: "cleanup",
        preview: {
          totals: { count: 2, size_bytes: 2048 },
          by_tool: [{ tool: "claude", count: 2, size_bytes: 2048 }],
          undoable: { count: 2, total: 2 },
          coverage: { covered: 3, total: 3, scope: "scope_fixture" },
          sessions: [{
            tool: "claude", ref: "fsr_fixture", title: "旧会话",
            project: "/tmp/project", updated: "2026-01-01T00:00:00Z",
            reason: "测试清理", undoable: true,
          }],
          excluded: [{ tool: "claude", ref: "fsr_keep", cause: "pinned" }],
        },
      },
    }}
    onApprove={() => {}}
    onDismiss={() => {}}
  />);

  expect(screen.getByText("askferry:cleanup.previewTitle")).toBeTruthy();
  expect(screen.getByText("askferry:cleanup.excluded")).toBeTruthy();
  expect(screen.getByText("fsr_keep")).toBeTruthy();
  fireEvent.click(screen.getByText("askferry:cleanup.showSessions"));
  expect(screen.getByText("旧会话")).toBeTruthy();
  expect(screen.getByText("测试清理", { exact: false })).toBeTruthy();
});
