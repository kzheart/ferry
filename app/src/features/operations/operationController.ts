import {
  type OperationKind,
  type OperationStatus,
  OPERATION_PLAN_ID_PREFIX,
  OPERATION_SUCCESS_STATUS,
  OPERATION_TERMINAL_STATUSES,
} from "../../api/contract/generated/operations.js";

export type OperationInput = {
  kind: OperationKind;
} & Record<string, unknown>;

export interface OperationPlan {
  plan_id: string;
  kind?: OperationKind;
  status?: OperationStatus;
  [key: string]: unknown;
}

export interface OperationState {
  plan_id: string;
  status: OperationStatus;
  error_type?: string;
  result?: unknown;
  [key: string]: unknown;
}

interface OperationClient {
  plan(input: OperationInput): Promise<OperationPlan>;
  apply(planId: string): Promise<OperationState>;
  status(planId: string): Promise<OperationState>;
  cancel(planId: string): Promise<unknown>;
}

interface ApplyOptions {
  onStatus?: (state: OperationState) => void;
}

interface OperationControllerOptions extends OperationClient {
  pause?: (milliseconds: number) => Promise<void>;
  pollInterval?: number;
}

const TERMINAL_STATUSES = new Set<OperationStatus>(
  OPERATION_TERMINAL_STATUSES,
);

const wait = (milliseconds: number) =>
  new Promise<void>(resolve => globalThis.setTimeout(resolve, milliseconds));

export class OperationNotAppliedError extends Error {
  readonly code = "operation.not_applied";
  readonly params: {
    plan_id: string;
    status: OperationStatus;
    error_type: string;
  };

  constructor(planId: string, status: OperationStatus, errorType = "") {
    super(`operation.not_applied: ${status}`);
    this.name = "OperationNotAppliedError";
    this.params = {
      plan_id: planId,
      status,
      error_type: errorType,
    };
  }
}

export class OperationController {
  private readonly client: OperationClient;
  private readonly pause: (milliseconds: number) => Promise<void>;
  private readonly pollInterval: number;

  constructor({
    plan,
    apply,
    status,
    cancel,
    pause = wait,
    pollInterval = 125,
  }: OperationControllerOptions) {
    this.client = { plan, apply, status, cancel };
    this.pause = pause;
    this.pollInterval = pollInterval;
  }

  plan(input: OperationInput) {
    return this.client.plan(input);
  }

  cancel(planId: string) {
    return this.client.cancel(planId);
  }

  async apply(
    planOrId: OperationPlan | string,
    { onStatus }: ApplyOptions = {},
  ) {
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

  async execute(input: OperationInput, options: ApplyOptions = {}) {
    const plan = await this.plan(input);
    return this.apply(plan, options);
  }
}
