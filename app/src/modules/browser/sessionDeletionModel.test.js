import assert from "node:assert/strict";
import test from "node:test";

import {
  applyPreparedDeletion,
  deleteNeedsConfirmation,
  prepareSessionDeletion,
  prepareSessionDeletions,
  summarizePreparedDeletions,
} from "./sessionDeletionModel.js";

const session = (id, treeCount = 1) => ({
  id,
  ref: `fsr_${id}`,
  tool: "fixture",
  tree_count: treeCount,
});

test("单会话删除复用已生成的 plan", async () => {
  const plan = {
    plan_id: "op_delete",
    preview: { undoable: true },
  };
  const calls = [];
  const client = {
    plan: async input => {
      calls.push(["plan", input]);
      return plan;
    },
    apply: async value => {
      calls.push(["apply", value]);
      return { result: { ok: true } };
    },
  };

  const prepared = await prepareSessionDeletion(session("one"), client);
  await applyPreparedDeletion(prepared, client);

  assert.strictEqual(prepared.plan, plan);
  assert.strictEqual(calls[1][1], plan);
  assert.equal(calls.filter(([name]) => name === "plan").length, 1);
});

test("仅可撤销且没有子会话的删除无需确认", () => {
  const undoable = {
    session: session("one"),
    plan: { preview: { undoable: true } },
  };
  const withChildren = {
    session: session("tree", 3),
    plan: { preview: { undoable: true } },
  };
  const irreversible = {
    session: session("final"),
    plan: { preview: { undoable: false } },
  };

  assert.equal(deleteNeedsConfirmation(undoable), false);
  assert.equal(deleteNeedsConfirmation(withChildren), true);
  assert.equal(deleteNeedsConfirmation(irreversible), true);
});

test("批量计划失败时取消已生成的计划且不返回半成品", async () => {
  const cancelled = [];
  const client = {
    plan: async input => {
      if (input.ref === "fsr_bad") throw new Error("cannot plan");
      return {
        plan_id: `op_${input.ref}`,
        preview: { undoable: input.ref !== "fsr_final" },
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
        plan_id: `op_${input.ref}`,
        preview: { undoable: true },
      };
      calls.push(["plan", plan]);
      return plan;
    },
    apply: async plan => {
      calls.push(["apply", plan]);
      return { result: { ok: true } };
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

test("批量确认摘要由 plan.preview.undoable 计算", () => {
  const prepared = [
    { session: session("one"), plan: { preview: { undoable: true } } },
    { session: session("two"), plan: { preview: { undoable: true } } },
    { session: session("three"), plan: { preview: { undoable: false } } },
  ];

  assert.deepEqual(summarizePreparedDeletions(prepared), {
    total: 3,
    undoable: 2,
    irreversible: 1,
  });
});
