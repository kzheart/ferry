import { test } from "vitest";
import assert from "node:assert/strict";
import { buildRoleBundle, parseRoleBundle, planRoleImport, roleBundleFileName }
  from "./roleBundle.js";

const role = (id, extra = {}) => ({
  id, name: "审阅者", persona: "只陈述证据。", tools: ["session_read"],
  skills: ["code-review"], apply_policy: "manual", builtin: false, ...extra,
});

test("导出剥掉运行时算出来的 builtin,保留图标与配色", () => {
  const bundle = buildRoleBundle([role("reader", { icon: "code", color: "violet" })],
    new Date("2026-07-25T00:00:00Z"));
  assert.equal(bundle.kind, "ferry.roles");
  assert.equal(bundle.exported_at, "2026-07-25T00:00:00.000Z");
  assert.deepEqual(bundle.roles[0], {
    id: "reader", name: "审阅者", icon: "code", color: "violet",
    persona: "只陈述证据。", tools: ["session_read"],
    skills: ["code-review"], apply_policy: "manual",
  });
  // allow_bash 已经不是角色字段:bash 现在只是 tools 里的一项
  assert.equal("allow_bash" in bundle.roles[0], false);
});

test("文件名单个角色用角色 ID,多个用 roles", () => {
  assert.equal(roleBundleFileName([role("reader")]), "ferry-reader.json");
  assert.equal(roleBundleFileName([role("a"), role("b")]), "ferry-roles.json");
});

test("解析挡掉非 bundle 与不支持的版本,同时接受裸 roles.json", () => {
  assert.throws(() => parseRoleBundle("{"), /notJson/);
  assert.throws(() => parseRoleBundle('{"kind":"other","roles":[]}'), /notBundle/);
  assert.throws(() => parseRoleBundle('{"kind":"ferry.roles","schema_version":9,"roles":[{}]}'),
    /badVersion/);
  assert.throws(() => parseRoleBundle('{"kind":"ferry.roles","roles":[]}'), /empty/);
  assert.equal(
    parseRoleBundle(JSON.stringify({ schema_version: 1, roles: [role("reader")] }))[0].id,
    "reader");
});

test("导入时与已有角色和同批次内部都要去重 ID", () => {
  const plan = planRoleImport([role("reader"), role("reader"), role("fresh")],
    ["default", "reader"]);
  assert.deepEqual(plan.map(item => item.role.id), ["reader-2", "reader-3", "fresh"]);
  assert.deepEqual(plan.map(item => item.renamedFrom), ["reader", "reader", null]);
});
