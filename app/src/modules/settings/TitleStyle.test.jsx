// 标题风格分区:读回已存风格、保存归一化后的值、「试一试」只出预览不写回。
import assert from "node:assert/strict";
import { test, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";

const engineCalls = [];
let saved = null;

vi.mock("../../platform/desktop/client.js", async (importOriginal) => ({
  ...(await importOriginal()),
  engine: async (method, params) => {
    engineCalls.push([method, params]);
    if (method === "title_style.get") {
      return { style: { preset: "bracket", language: "en", max_chars: 20, type_prefix: true,
        types: [{ emoji: "✨", zh: "实现", en: "feat" }], instructions: "", examples: [] } };
    }
    if (method === "title_style.set") {
      saved = params.style;
      return { style: params.style };
    }
    return { sessions: [{ tool: "claude", ref: "fsr_a", title: "旧 A", title_source: "native" }],
      errors: [], style: {} };
  },
  runtime: async () => ({
    items: [{ tool: "claude", ref: "fsr_a", title: "✨ 实现甲", skip: false, before: "旧 A" }],
    model: { provider_id: "p", model: "m" },
  }),
}));

const { default: TitleStyle } = await import("./TitleStyle.jsx");

const clickText = async text => {
  const button = [...document.querySelectorAll("button")]
    .find(node => node.textContent.includes(text));
  assert.ok(button, `找不到按钮：${text}`);
  await act(async () => { button.click(); });
};

test("读回已保存的风格,保存时去掉空的类型行", async () => {
  engineCalls.length = 0;
  saved = null;
  render(<TitleStyle sessions={[]} />);
  await act(async () => {});

  assert.equal(engineCalls[0][0], "title_style.get");
  assert.equal(screen.getByLabelText("settings:titleStyle.maxChars").value, "20");

  await clickText("settings:titleStyle.addType");
  await clickText("settings:titleStyle.save");
  assert.equal(saved.types.length, 1);
  assert.equal(saved.preset, "bracket");
  assert.ok(screen.getByText("settings:titleStyle.saved"));
});

test("试一试用当前设置生成预览,不写回任何会话", async () => {
  engineCalls.length = 0;
  render(<TitleStyle sessions={[{ tool: "claude", ref: "fsr_a", id: "a" }]} />);
  await act(async () => {});
  await clickText("settings:titleStyle.tryOut");

  assert.ok(screen.getByText("✨ 实现甲"));
  assert.deepEqual(
    engineCalls.map(([method]) => method),
    ["title_style.get", "title_evidence"],
  );
});
