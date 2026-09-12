import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { ApprovalCard } from "./AgentApprovalCard.jsx";

afterEach(() => vi.useRealTimers());
const proposal = (overrides = {}) => ({
  status: "pending",
  operation: { kind: "metadata", summary: "更新会话标题", risk: "low", affected_refs: ["session-1"] },
  ...overrides,
});

test("截止时立即移除批准动作，不会自动批准", () => {
  vi.useFakeTimers();
  vi.setSystemTime(10000);
  const onApprove = vi.fn();
  render(<ApprovalCard item={proposal({ operation: { expires_at: 11500, summary: "测试" } })} onApprove={onApprove} />);
  expect(screen.getByText("0:02")).toBeTruthy();
  act(() => vi.advanceTimersByTime(1500));
  expect(screen.queryByText("askferry:approval.approve")).toBe(null);
  expect(screen.queryByText("askferry:approval.reject")).toBe(null);
  expect(screen.getByText("askferry:approval.expired")).toBeTruthy();
  expect(onApprove).not.toHaveBeenCalled();
});

test("时钟已经过期但计时器未触发时，点击仍然不能批准", () => {
  vi.useFakeTimers();
  vi.setSystemTime(10000);
  const onApprove = vi.fn();
  render(<ApprovalCard item={proposal({ operation: { expires_at: 11000 } })} onApprove={onApprove} />);
  vi.setSystemTime(11000);
  fireEvent.click(screen.getByText("askferry:approval.approve"));
  expect(onApprove).not.toHaveBeenCalled();
  expect(screen.getByText("askferry:approval.expired")).toBeTruthy();
});

test("有效提议由用户批准或拒绝，保留风险与影响信息", () => {
  const onApprove = vi.fn();
  const onDismiss = vi.fn();
  render(<ApprovalCard item={proposal()} onApprove={onApprove} onDismiss={onDismiss} />);
  expect(screen.getByText("更新会话标题")).toBeTruthy();
  expect(screen.getByText("askferry:approval.affected")).toBeTruthy();
  expect(screen.getByText("askferry:approval.risk")).toBeTruthy();
  fireEvent.click(screen.getByText("askferry:approval.approve"));
  expect(onApprove).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByText("askferry:approval.reject"));
  expect(onDismiss).toHaveBeenCalledOnce();
});

test.each(["applied", "applying", "failed", "dismissed"])("%s 不再显示审批动作", status => {
  render(<ApprovalCard item={proposal({ status })} />);
  expect(screen.getByText(`askferry:approval.${status}`)).toBeTruthy();
  expect(screen.queryByText("askferry:approval.approve")).toBe(null);
});

test("卸载后清理倒计时", () => {
  vi.useFakeTimers();
  vi.setSystemTime(10000);
  const { unmount } = render(<ApprovalCard item={proposal({ operation: { expires_at: 20000 } })} />);
  expect(vi.getTimerCount()).toBe(1);
  unmount();
  expect(vi.getTimerCount()).toBe(0);
});
