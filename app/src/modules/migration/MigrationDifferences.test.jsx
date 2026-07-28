// 差异卡的价值在于「这次调用被改成了什么样」,而不是重复一遍保真度标签。
// 这些用例守的就是卡片上必须出现的信息:源→目标的落点、被丢掉的参数、展开后的前后对照。
import { test } from "vitest";
import assert from "node:assert/strict";
import { fireEvent, render, screen } from "@testing-library/react";

import DifferenceReview from "./MigrationDifferences.jsx";

const t = (key, options) => {
  const suffix = options?.fields ? `:${options.fields}` : "";
  return `${key.split(".").pop()}${suffix}`;
};

function review(item) {
  return render(<DifferenceReview
    preview={{ differences: { items: [item], counts: { total: 1 } } }}
    t={t} onBack={() => {}} onLocate={() => {}} />);
}

const transformed = {
  id: "d1", kind: "degraded", fidelity: "transformed", reason_code: "tool_transformed",
  node_key: "n:0", node_path: "0", node_title: "Demo", round_index: 3, ignored_fields: [],
  source: { kind: "tool", label: "Bash", detail: "…",
    parts: { input: '{"command": "ls"}', output: "a.txt" } },
  target: { kind: "tool", label: "bash", detail: "…",
    parts: { input: '{"command": "ls"}', output: "a.txt" } },
};

test("转换卡展示源工具到目标工具的落点,不再复述保真度文案", () => {
  review(transformed);
  assert.ok(screen.getByText("Bash"));
  assert.ok(screen.getByText("bash"));
  assert.equal(screen.queryByText("tool_transformed"), null);
});

test("展开后给出原始与迁移后的参数、结果对照", () => {
  review(transformed);
  fireEvent.click(screen.getByRole("button", { expanded: false }));
  assert.equal(screen.getAllByText("params").length, 2);
  assert.equal(screen.getAllByText("result").length, 2);
  assert.equal(screen.getAllByText('{"command": "ls"}').length, 2);
});

test("有损卡直接点名被丢掉的参数", () => {
  review({ ...transformed, id: "d2", fidelity: "lossy",
    reason_code: "unsupported_tool_fields", ignored_fields: ["timeout_ms", "background"] });
  assert.ok(screen.getByText("lostFields:timeout_ms, background"));
});

test("退化成叙述时不谎报参数丢失:参数其实都还在叙述文本里", () => {
  review({ ...transformed, id: "d4", fidelity: "narrated", reason_code: "tool_to_history",
    ignored_fields: ["namespace", "name", "input"],
    target: { kind: "text", label: "history", detail: "[History: tool exec …]" } });
  assert.equal(screen.queryByText(/lostFields/), null);
  assert.ok(screen.getByText("tool_to_history"));
});

test("丢弃卡标出目标端没有落点", () => {
  review({ ...transformed, id: "d3", kind: "dropped", fidelity: "dropped",
    reason_code: "tool_unsupported", target: null });
  assert.ok(screen.getByText("noTarget"));
});
