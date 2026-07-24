import {
  OPERATION_PLAN_ID_PREFIX,
  OPERATION_SUCCESS_STATUS,
  OPERATION_TERMINAL_STATUSES,
} from "../../api/contract/generated/operations.js";

const TERMINAL_STATUSES = new Set(OPERATION_TERMINAL_STATUSES);

const wait = milliseconds =>
  new Promise(resolve => globalThis.setTimeout(resolve, milliseconds));

export class OperationNotAppliedError extends Error {
  constructor(planId, status, errorType = "") {
    super(`operation.not_applied: ${status}`);
    this.name = "OperationNotAppliedError";
    this.code = "operation.not_applied";
    this.params = {
      plan_id: planId,
      status,
      error_type: errorType,
    };
  }
}

export class OperationController {
  constructor({
    plan,
    apply,
    status,
    cancel,
    pause = wait,
    pollInterval = 125,
  }) {
    if (![plan, apply, status, cancel].every(value => typeof value === "function")) {
      throw new TypeError("operation client methods are required");
    }
    this.client = { plan, apply, status, cancel };
    this.pause = pause;
    this.pollInterval = pollInterval;
  }

  plan(input) {
    return this.client.plan(input);
  }

  cancel(planId) {
    return this.client.cancel(planId);
  }

  async apply(planOrId, { onStatus } = {}) {
    const planId =
      typeof planOrId === "string" ? planOrId : planOrId?.plan_id;
    if (!planId?.startsWith(OPERATION_PLAN_ID_PREFIX)) {
      throw new Error("operation plan_id is required");
    }
    let current = await this.client.apply(planId);
    onStatus?.(current);
    while (!TERMINAL_STATUSES.has(current.status)) {
      await this.pause(this.pollInterval);
      current = await this.client.status(planId);
      onStatus?.(current);
    }
    if (current.status !== OPERATION_SUCCESS_STATUS) {
      throw new OperationNotAppliedError(
        planId,
        current.status,
        current.error_type,
      );
    }
    return current;
  }

  async execute(input, options) {
    const plan = await this.plan(input);
    return this.apply(plan, options);
  }
}
