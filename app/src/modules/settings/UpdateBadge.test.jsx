// 更新入口挂在导航栏「设置」行的行尾,和那一行共享点击区。这里守两条:
// 只在有事可做时出现,以及点图标只触发更新、不会顺带把设置页打开。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render, screen } from "@testing-library/react";

import { UpdateBadge } from "./UpdateBadge.jsx";

const noop = () => {};

// 整行是可点的,徽标嵌在里面——真实结构照搬,否则测不出冒泡
const inRow = (props, onRowClick = noop) => render(
  <button type="button" onClick={onRowClick}>
    <span>设置</span>
    <UpdateBadge phase="idle" version={undefined} progress={null} onStart={noop} {...props} />
  </button>,
);

test.each(["idle", "checking", "upToDate", "error"])(
  "没有可下载的更新时行尾不留图标位:%s", phase => {
    inRow({ phase });
    assert.equal(screen.queryByRole("button", { name: /updates\./ }), null);
  });

test("有更新时点图标只开始更新,不把设置页一起打开", () => {
  const calls = [];
  inRow(
    { phase: "available", version: "0.8.3", onStart: () => calls.push("update") },
    () => calls.push("settings"),
  );

  fireEvent.click(screen.getByRole("button", { name: /badgeAvailable/ }));
  assert.deepEqual(calls, ["update"], "冒泡到整行就会误开设置页");
});

test("下载与安装期间图标不可点,避免重复触发同一次更新", () => {
  const calls = [];
  const { unmount } = inRow({ phase: "downloading", progress: 0.4,
    onStart: () => calls.push("update") });
  fireEvent.click(screen.getByRole("button", { name: /badgeDownloading/ }));
  unmount();

  inRow({ phase: "installing", onStart: () => calls.push("update") });
  fireEvent.click(screen.getByRole("button", { name: /badgeInstalling/ }));
  assert.deepEqual(calls, []);
});
