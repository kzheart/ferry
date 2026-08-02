import { act, fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { AgentChoiceCard } from "./AgentChoiceCard.jsx";

const card = (overrides = {}) => ({
  kind: "choice",
  requestId: "req-1",
  callId: "call-1",
  question: "清理哪些会话?",
  options: [
    { label: "旧会话", recommended: true },
    { label: "全部" },
    { label: "都不要" },
  ],
  multiSelect: false,
  allowCustom: false,
  status: "pending",
  answered: false,
  selected: [],
  customText: "",
  ...overrides,
});

const submitButton = () => screen.getByText("askferry:choice.submit").closest("button");
const skipButton = () => screen.getByText("askferry:choice.skip").closest("button");

test("单选只提交一个选项,再点另一个会替换掉前一个", () => {
  const onRespond = vi.fn(async () => {});
  render(<AgentChoiceCard item={card()} onRespond={onRespond} />);

  // 没选之前不能提交
  expect(submitButton().disabled).toBe(true);

  fireEvent.click(screen.getByText("旧会话"));
  fireEvent.click(screen.getByText("全部"));
  fireEvent.click(submitButton());

  expect(onRespond).toHaveBeenCalledWith({
    answered: true, selected: ["全部"], custom_text: "",
  });
});

test("多选累积选项,自由输入随答案一起提交", () => {
  const onRespond = vi.fn(async () => {});
  render(<AgentChoiceCard
    item={card({ multiSelect: true, allowCustom: true })}
    onRespond={onRespond} />);

  fireEvent.click(screen.getByText("旧会话"));
  fireEvent.click(screen.getByText("全部"));
  // 再点一次取消
  fireEvent.click(screen.getByText("全部"));
  fireEvent.change(screen.getByPlaceholderText("askferry:choice.customPlaceholder"),
    { target: { value: "  只删 7 天前的  " } });
  fireEvent.click(submitButton());

  expect(onRespond).toHaveBeenCalledWith({
    answered: true, selected: ["旧会话"], custom_text: "只删 7 天前的",
  });
});

test("提交中按钮仍在,只是禁用并显示提交中文案", async () => {
  let release;
  const onRespond = vi.fn(() => new Promise(resolve => { release = resolve; }));
  render(<AgentChoiceCard item={card()} onRespond={onRespond} />);

  fireEvent.click(screen.getByText("旧会话"));
  await act(async () => { fireEvent.click(submitButton()); });

  // 按钮区不能在等待期间整块消失,否则用户点完什么反馈都看不到
  const button = screen.getByText("askferry:choice.submitting").closest("button");
  expect(button).toBeTruthy();
  expect(button.disabled).toBe(true);
  expect(skipButton().disabled).toBe(true);
  expect(screen.queryByText("askferry:choice.submit")).toBe(null);

  await act(async () => { release(); });
  expect(screen.getByText("askferry:choice.submit")).toBeTruthy();
});

test("跳过按钮提交 answered:false,不夹带任何选择", () => {
  const onRespond = vi.fn(async () => {});
  render(<AgentChoiceCard item={card()} onRespond={onRespond} />);

  fireEvent.click(screen.getByText("旧会话"));
  fireEvent.click(skipButton());

  expect(onRespond).toHaveBeenCalledWith({
    answered: false, selected: [], custom_text: "",
  });
});

test("已作答的卡片只读:没有按钮,输入被禁用,回显既有答案", () => {
  const onRespond = vi.fn(async () => {});
  render(<AgentChoiceCard
    item={card({ status: "answered", answered: true, selected: ["全部"],
      customText: "备注", allowCustom: true })}
    onRespond={onRespond} />);

  expect(screen.queryByText("askferry:choice.submit")).toBe(null);
  expect(screen.queryByText("askferry:choice.skip")).toBe(null);
  expect(screen.getByText("askferry:choice.answered")).toBeTruthy();
  expect(screen.getByPlaceholderText("askferry:choice.customPlaceholder").value)
    .toBe("备注");

  // 只读态点选项不应该产生任何应答
  fireEvent.click(screen.getByText("旧会话"));
  expect(onRespond).not.toHaveBeenCalled();
});

test("未作答态给出运行已结束的说明,同样不可再操作", () => {
  render(<AgentChoiceCard item={card({ status: "unanswered" })}
    onRespond={vi.fn()} />);

  expect(screen.getByText("askferry:choice.unanswered")).toBeTruthy();
  expect(screen.getByText("askferry:choice.noAnswer")).toBeTruthy();
  expect(screen.queryByText("askferry:choice.submit")).toBe(null);
});
