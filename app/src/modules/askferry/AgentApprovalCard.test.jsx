import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { ApprovalCard } from "./AgentApprovalCard.jsx";

test("delete approval card renders totals, permanent warning and excluded entries", () => {
  render(<ApprovalCard
    item={{
      status: "pending",
      operation: {
        plan_id: "op_delete_fixture",
        kind: "delete",
        preview: {
          tool: "claude",
          totals: { count: 1, size_bytes: 2048 },
          permanent: true,
          sessions: [{
            tool: "claude", ref: "fsr_fixture", title: "旧会话",
            project: "/tmp/project", updated: "2026-01-01T00:00:00Z",
          }],
          excluded: [{
            tool: "claude", ref: "fsr_keep", title: "钉住的会话",
            cause: "pinned",
          }],
        },
      },
    }}
    onApprove={() => {}}
    onDismiss={() => {}}
  />);

  expect(screen.getByText("askferry:deletion.previewTitle")).toBeTruthy();
  expect(screen.getByText("askferry:deletion.permanent")).toBeTruthy();
  expect(screen.getByText("askferry:deletion.excluded")).toBeTruthy();
  expect(screen.getByText("钉住的会话")).toBeTruthy();
  expect(screen.getByText("askferry:deletion.causePinned")).toBeTruthy();
  fireEvent.click(screen.getByText("askferry:deletion.showSessions"));
  expect(screen.getByText("旧会话")).toBeTruthy();
  expect(screen.getByText("/tmp/project", { exact: false })).toBeTruthy();
});
