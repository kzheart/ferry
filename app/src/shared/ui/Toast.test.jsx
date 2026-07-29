// 提示条的消隐策略:成功说完就让路,失败和进行中必须留在原地。
// 带撤销的成功提示要多留一会儿,悬停期间不能在用户眼皮底下消失。
import { afterEach, beforeEach, test, vi } from "vitest";
import assert from "node:assert/strict";
import { act, fireEvent, render, screen } from "@testing-library/react";

import { Toast } from "./Toast.jsx";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

const advance = ms => act(() => { vi.advanceTimersByTime(ms); });

function renderToast(toast) {
  let dismissed = 0;
  render(<Toast toast={toast} onDismiss={() => { dismissed += 1; }} />);
  return () => dismissed;
}

test("成功提示在 3.2s 后自动消隐", () => {
  const dismissed = renderToast({ kind: "ok", title: "已保存", desc: "元数据已写入" });

  advance(3100);
  assert.equal(dismissed(), 0);
  advance(200);
  assert.equal(dismissed(), 1);
});

test("失败提示不自动消隐,留给用户读错误", () => {
  const dismissed = renderToast({ kind: "fail", title: "删除失败", desc: "权限不足" });

  advance(60_000);
  assert.equal(dismissed(), 0);
});

test("进行中提示不自动消隐,由业务态驱动", () => {
  const dismissed = renderToast({ kind: "run", title: "删除中", desc: "正在写入" });

  advance(60_000);
  assert.equal(dismissed(), 0);
});

test("带撤销的成功提示留更久,别让人来不及点", () => {
  const dismissed = renderToast({
    kind: "ok", title: "已删除", desc: "可撤销",
    action: { label: "撤销", onClick: () => {} },
  });

  advance(3400);
  assert.equal(dismissed(), 0, "撤销还够得着");
  advance(3800);
  assert.equal(dismissed(), 1);
});

test("鼠标悬停期间暂停计时,移开后重新计时", () => {
  let dismissed = 0;
  render(
    <Toast
      toast={{ kind: "ok", title: "已保存", desc: "元数据已写入" }}
      onDismiss={() => { dismissed += 1; }}
    />,
  );
  const box = screen.getByRole("status");

  advance(2000);
  fireEvent.mouseEnter(box);
  advance(60_000);
  assert.equal(dismissed, 0, "悬停期间不消失");

  fireEvent.mouseLeave(box);
  advance(3300);
  assert.equal(dismissed, 1);
});

test("换一条提示会重新计时,不会继承上一条已走过的时间", () => {
  let dismissed = 0;
  const onDismiss = () => { dismissed += 1; };
  const { rerender } = render(
    <Toast toast={{ kind: "ok", title: "第一条", desc: "" }} onDismiss={onDismiss} />,
  );

  advance(3000);
  rerender(<Toast toast={{ kind: "ok", title: "第二条", desc: "" }} onDismiss={onDismiss} />);
  advance(1000);
  assert.equal(dismissed, 0, "第二条应从头计时");
  advance(2400);
  assert.equal(dismissed, 1);
});

// ---- 外观:整块染色是 Ferry 里唯一的一处,改完三种状态共用中性表面 ----

const box = () => screen.getByRole("status");

test("三种状态共用中性表面,不再按 kind 铺色", () => {
  const surfaces = ["ok", "fail", "run"].map(kind => {
    const view = render(<Toast toast={{ kind, title: "t", desc: "d" }} onDismiss={() => {}} />);
    const bg = box().style.background;
    view.unmount();
    return bg;
  });

  assert.deepEqual(surfaces, ["var(--surface)", "var(--surface)", "var(--surface)"]);
});

test("失败态靠一圈细描边比成功重一档,而不是靠粉底", () => {
  const fail = render(<Toast toast={{ kind: "fail", title: "t", desc: "d" }} onDismiss={() => {}} />);
  const failShadow = box().style.boxShadow;
  fail.unmount();
  render(<Toast toast={{ kind: "ok", title: "t", desc: "d" }} onDismiss={() => {}} />);

  assert.ok(failShadow.includes("var(--err-line)"), "失败态该有 err-line 描边");
  assert.ok(failShadow.includes("var(--shadow-menu)"), "投影用菜单档,不是大面板档");
  assert.equal(box().style.boxShadow, "var(--shadow-menu)", "成功态只有菜单档投影");
});

test("关闭键是有无障碍名称的图标按钮,不再是与失败图标同形的文字 ×", () => {
  render(<Toast toast={{ kind: "fail", title: "删除失败", desc: "d", dismissLabel: "关闭" }}
    onDismiss={() => {}} />);

  const dismiss = screen.getByRole("button", { name: "关闭" });
  assert.ok(dismiss.querySelector("svg"), "应当是 SVG 图标");
  assert.equal(screen.queryByText("×"), null, "整条提示里不该再有文字 ×");
});

// ---- 倒计时细线:自动消隐的唯一预告 ----

const bar = () => box().querySelector(".ftoast-bar");

test("只有会自动消失的提示才画倒计时细线", () => {
  const fail = render(<Toast toast={{ kind: "fail", title: "t", desc: "d" }} onDismiss={() => {}} />);
  assert.equal(bar(), null, "失败不会自动消失,不该画");
  fail.unmount();

  const run = render(<Toast toast={{ kind: "run", title: "t", desc: "d" }} onDismiss={() => {}} />);
  assert.equal(bar(), null, "进行中由业务态驱动,不该画");
  run.unmount();

  render(<Toast toast={{ kind: "ok", title: "t", desc: "d" }} onDismiss={() => {}} />);
  assert.ok(bar());
});

test("细线的时长与实际消隐时长一致,带撤销的那条更长", () => {
  const plain = render(<Toast toast={{ kind: "ok", title: "t", desc: "d" }} onDismiss={() => {}} />);
  assert.ok(bar().style.animation.includes("3200ms"));
  plain.unmount();

  render(<Toast toast={{ kind: "ok", title: "t", desc: "d", action: { label: "撤销", onClick: () => {} } }}
    onDismiss={() => {}} />);
  assert.ok(bar().style.animation.includes("7000ms"));
});

test("悬停时细线停成满格:移开确实是重新计时,不能冻在半路谎报进度", () => {
  render(<Toast toast={{ kind: "ok", title: "t", desc: "d" }} onDismiss={() => {}} />);

  advance(2000);
  fireEvent.mouseEnter(box());
  assert.equal(bar().style.animation, "", "悬停期间不跑动画,细线保持满格");

  fireEvent.mouseLeave(box());
  assert.ok(bar().style.animation.includes("3200ms"), "移开后从头跑满整段");
});
