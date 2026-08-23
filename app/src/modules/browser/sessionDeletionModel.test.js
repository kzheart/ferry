import assert from "node:assert/strict";
import { test } from "vitest";

import {
  applyPreparedDeletion,
  deleteIsBlocked,
  prepareSessionDeletion,
  prepareSessionDeletions,
} from "./sessionDeletionModel.js";

const session = (id, treeCount = 1) => ({
  id,
  ref: `fsr_${id}`,
  tool: "fixture",
  tree_count: treeCount,
});

test("单会话删除以 refs 数组下计划并复用已生成的 plan", async () => {
  const plan = {
    plan_id: "op_delete",
    preview: { totals: { count: 1 }, excluded: [] },
  };
  const calls = [];
  const client = {
    plan: async input => {
      calls.push(["plan", input]);
      return plan;
    },
    apply: async value => {
      calls.push(["apply", value]);
      return { result: { succeeded: [{ ref: "fsr_one" }] } };
    },
  };

  const prepared = await prepareSessionDeletion(session("one"), client);
  await applyPreparedDeletion(prepared, client);

  assert.deepEqual(calls[0][1], {
    kind: "delete",
    tool: "fixture",
    refs: ["fsr_one"],
  });
  assert.strictEqual(prepared.plan, plan);
  assert.strictEqual(calls[1][1], plan);
  assert.equal(calls.filter(([name]) => name === "plan").length, 1);
});

test("受保护会话在计划期被排除并标记为 blocked", () => {
  const blocked = {
    session: session("pinned"),
    plan: { preview: { sessions: [], excluded: [{ cause: "pinned" }] } },
  };
  const deletable = {
    session: session("plain"),
    plan: { preview: { sessions: [{}], excluded: [] } },
  };

  assert.equal(deleteIsBlocked(blocked), true);
  assert.equal(deleteIsBlocked(deletable), false);
});

test("批量计划失败时取消已生成的计划且不返回半成品", async () => {
  const cancelled = [];
  const client = {
    plan: async input => {
      if (input.refs[0] === "fsr_bad") throw new Error("cannot plan");
      return {
        plan_id: `op_${input.refs[0]}`,
        preview: { excluded: [] },
      };
    },
    cancel: async planId => {
      cancelled.push(planId);
    },
  };

  await assert.rejects(
    () => prepareSessionDeletions(
      [session("one"), session("final"), session("bad")],
      client,
    ),
    /cannot plan/,
  );
  assert.deepEqual(cancelled.sort(), ["op_fsr_final", "op_fsr_one"]);
});

test("批量删除先完成全部计划并复用各自的 plan", async () => {
  const calls = [];
  const client = {
    plan: async input => {
      const plan = {
        plan_id: `op_${input.refs[0]}`,
        preview: { excluded: [] },
      };
      calls.push(["plan", plan]);
      return plan;
    },
    apply: async plan => {
      calls.push(["apply", plan]);
      return { result: { succeeded: [{}] } };
    },
  };

  const prepared = await prepareSessionDeletions(
    [session("one"), session("two")],
    client,
  );
  for (const target of prepared) {
    await applyPreparedDeletion(target, client);
  }

  assert.deepEqual(calls.map(([name]) => name), [
    "plan",
    "plan",
    "apply",
    "apply",
  ]);
  assert.strictEqual(calls[2][1], prepared[0].plan);
  assert.strictEqual(calls[3][1], prepared[1].plan);
});

