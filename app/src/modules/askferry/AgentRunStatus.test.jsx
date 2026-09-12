import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { AgentRunStatus, classifyRunFailure } from "./AgentRunStatus.jsx";

test.each([
  ["provider request failed: Connection error.", "connection"],
  ["connect ECONNREFUSED 127.0.0.1:8318", "connection"],
  ["invalid_api_key", "auth"],
  ["401 Unauthorized", "auth"],
  ["rate_limit_exceeded", "rateLimit"],
  ["Too many requests", "rateLimit"],
  ["Failed to read file 401.txt", "generic"],
  ["Tool returned invalid result", "generic"],
  ["The connection pool was full", "generic"],
  [undefined, "generic"],
])("分类仅识别明确的错误信号：%s", (message, expected) => {
  expect(classifyRunFailure(message)).toBe(expected);
});

test("错误详情默认折叠，保留可选取的原始错误而非堆在标题中", () => {
  const message = "provider request failed: Connection error.";
  const { container } = render(<AgentRunStatus item={{ type: "run.failed", message }} />);
  expect(screen.getByText("askferry:chat.failure.connectionTitle")).toBeTruthy();
  expect(screen.getByText("askferry:chat.failure.connectionHint")).toBeTruthy();
  const details = container.querySelector("details");
  expect(details.open).toBe(false);
  expect(details.querySelector("summary").textContent).toBe("askferry:chat.failure.details");
  expect(details.querySelector("pre").textContent).toBe(message);
  expect(details.querySelector("pre").classList.contains("selectable")).toBe(true);
  expect(container.querySelector("button")).toBe(null);
});

test("没有详情时不显示空折叠项", () => {
  const { container } = render(<AgentRunStatus item={{ type: "run.failed" }} />);
  expect(screen.getByText("askferry:chat.failure.genericTitle")).toBeTruthy();
  expect(container.querySelector("details")).toBe(null);
});

test.each([
  ["run.cancelled", "runCancelled"],
  ["run.interrupted", "runInterrupted"],
  ["context.compacted", "contextCompacted"],
])("%s 用轻量状态行展示", (type, key) => {
  const { container } = render(<AgentRunStatus item={{ type }} />);
  expect(screen.getByText(`askferry:chat.${key}`)).toBeTruthy();
  expect(container.querySelector(".agent-run-feedback")).toBe(null);
});
