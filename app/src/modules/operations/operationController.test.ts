import assert from "node:assert/strict";
import test from "node:test";

import type { OperationStatus }
  from "../../shared/contracts/generated/operations.js";
import { OperationController } from "./operationController.js";

test("controller owns plan, apply and status polling", async () => {
  const calls: Array<[string, unknown]> = [];
  const statuses: OperationStatus[] = [];
  const controller = new OperationController({
    plan: async input => {
      calls.push(["plan", input]);
      return { plan_id: "op_fixture" };
    },
    apply: async planId => {
      calls.push(["apply", planId]);
      return { plan_id: planId, status: "queued" };
    },
    status: async planId => {
      calls.push(["status", planId]);
      return { plan_id: planId, status: "applied", result: { ok: true } };
    },
    cancel: async () => {},
    pause: async () => {},
  });

  const result = await controller.execute(
    { kind: "metadata", tool: "claude", ref: "fsr_fixture", patch: {} },
    { onStatus: state => statuses.push(state.status) },
  );

  assert.deepEqual(calls.map(([name]) => name), ["plan", "apply", "status"]);
  assert.deepEqual(statuses, ["queued", "applied"]);
  assert.deepEqual(result.result, { ok: true });
});

test("controller rejects non-applied terminal states", async () => {
  const controller = new OperationController({
    plan: async () => ({ plan_id: "op_fixture" }),
    apply: async planId => ({ plan_id: planId, status: "failed" }),
    status: async () => {
      throw new Error("status should not be called");
    },
    cancel: async () => {},
    pause: async () => {},
  });

  await assert.rejects(
    () => controller.apply("op_fixture"),
    /operation\.not_applied|failed/,
  );
});
