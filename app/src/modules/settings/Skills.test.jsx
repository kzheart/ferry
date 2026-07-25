import { expect, test } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { FerryRuntimeProvider } from "../../shared/capabilities/ferryRuntime.jsx";
import Skills from "./Skills.jsx";

const INSTALLED = {
  id: "code-review", name: "代码评审", description: "逐条核对变更",
  bytes: 2048, files: 2, originLabel: "Claude Code", broken: false,
};
const CANDIDATE = {
  candidateId: "claude:pdf-tools", name: "PDF 工具", description: "拆分与合并",
  source: "claude", path: "/home/u/.claude/skills/pdf-tools",
};

function harness(overrides = {}) {
  const calls = { import: [], global: [], delete: [] };
  const value = {
    skills: { skills: [INSTALLED], global: [], scan_sources: [] },
    skillCandidates: {
      candidates: [CANDIDATE],
      sources: [{ id: "claude", label: "Claude Code", path: "/home/u/.claude/skills",
        builtin: true, available: true }],
    },
    reloadSkills: async () => {},
    loadSkillCandidates: async () => {},
    readSkill: async () => ({ body: "# 正文" }),
    importSkill: async (params) => { calls.import.push(params); return { skill: INSTALLED }; },
    deleteSkill: async (id) => { calls.delete.push(id); },
    setGlobalSkills: async (ids) => { calls.global.push(ids); },
    addSkillSource: async () => {},
    importSkillFolder: async () => {},
    removeSkillSource: async () => {},
    ...overrides,
  };
  const view = render(
    <FerryRuntimeProvider value={value}><Skills /></FerryRuntimeProvider>);
  return { calls, view };
}

const clickText = async (text) => {
  const node = [...document.querySelectorAll("button")]
    .find((button) => button.textContent.trim() === text);
  expect(node, `找不到按钮：${text}`).toBeTruthy();
  await act(async () => { node.click(); });
};

const clickContaining = async (text) => {
  const node = [...document.querySelectorAll("button")]
    .find((button) => button.textContent.includes(text));
  expect(node, `找不到包含「${text}」的按钮`).toBeTruthy();
  await act(async () => { node.click(); });
};

/** 来源默认折叠,候选要先展开来源才看得到。 */
const expandSources = async () => {
  for (const node of document.querySelectorAll("button[aria-expanded=false]")) {
    await act(async () => { node.click(); });
  }
};

test("已导入区与可导入区分别渲染", async () => {
  harness();
  expect(screen.getByText("settings:skills.mine")).toBeTruthy();
  expect(screen.getByText("settings:skills.available")).toBeTruthy();
  expect(screen.getByText("代码评审")).toBeTruthy();
  await expandSources();
  expect(screen.getByText("PDF 工具")).toBeTruthy();
});

test("来源默认折叠,只露出目录名与候选数", async () => {
  harness();
  expect(screen.getByText("Claude Code")).toBeTruthy();
  expect(screen.getByText("1")).toBeTruthy();
  expect(screen.queryByText("PDF 工具")).toBeNull();
  await expandSources();
  expect(screen.getByText("PDF 工具")).toBeTruthy();
});

test("选中候选之后仍然能把来源折叠回去", async () => {
  harness();
  await expandSources();
  await clickContaining("PDF 工具");
  // 列表一处、详情标题一处
  expect(screen.getAllByText("PDF 工具")).toHaveLength(2);

  await act(async () => {
    document.querySelector("button[aria-expanded=true]").click();
  });
  // 折叠只收起列表,详情不受影响
  expect(screen.getAllByText("PDF 工具")).toHaveLength(1);
});

test("点候选的导入,入参带 candidate_id", async () => {
  const { calls } = harness();
  await expandSources();
  await clickContaining("PDF 工具");
  await clickText("settings:skills.import");
  expect(calls.import).toEqual([
    { candidate_id: "claude:pdf-tools", overwrite: false },
  ]);
});

test("候选详情不出现「设为通用」开关——没导入就不能配置", async () => {
  harness();
  await expandSources();
  await clickContaining("PDF 工具");
  expect(screen.queryByText("settings:skills.globalTitle")).toBeNull();
  expect(screen.getByText("settings:skills.import")).toBeTruthy();
});

test("已导入项打开通用开关,入参包含该 id", async () => {
  const { calls } = harness();
  await clickContaining("代码评审");
  expect(screen.getByText("settings:skills.globalTitle")).toBeTruthy();
  const toggle = document.querySelector('button[aria-pressed]');
  await act(async () => { toggle.click(); });
  expect(calls.global).toEqual([["code-review"]]);
});
