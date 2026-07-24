import type { ProviderHost } from "../providers/provider-host.js";
import { ProtocolError } from "../server/messages.js";
import { parseOrganizerInput } from "./organizer.js";
import {
  runOrganizationWorkflow,
  type OrganizationEngineMethod,
} from "./organization.js";

export interface OrganizationCoordinatorOptions {
  providerHost?: ProviderHost;
  newId: () => string;
  invokeEngine: (
    method: OrganizationEngineMethod,
    params: Record<string, unknown>,
    workflowId: string,
  ) => Promise<unknown>;
}

type OrganizationJobStatus = "running" | "completed" | "failed" | "cancelled";

interface OrganizationJob {
  jobId: string;
  key: string;
  status: OrganizationJobStatus;
  phase: "generating" | "committing";
  cancelled: boolean;
  result?: unknown;
  error?: { code: string; message: string };
}

class OrganizationCancelled extends Error {}

const MAX_RETAINED_JOBS = 100;

export class OrganizationCoordinator {
  private readonly jobs = new Map<string, OrganizationJob>();
  private readonly runningByInput = new Map<string, string>();

  constructor(private readonly options: OrganizationCoordinatorOptions) {}

  start(input: unknown) {
    if (!this.options.providerHost) {
      throw new ProtocolError(
        "unsupported",
        "organization generation unavailable",
      );
    }
    const key = this.inputKey(input);
    const runningId = this.runningByInput.get(key);
    if (runningId) return this.publicState(this.requireJob(runningId));

    this.prune();
    const job: OrganizationJob = {
      jobId: `org_${this.options.newId()}`.slice(0, 128),
      key,
      status: "running",
      phase: "generating",
      cancelled: false,
    };
    this.jobs.set(job.jobId, job);
    this.runningByInput.set(key, job.jobId);
    void this.run(job, input);
    return this.publicState(job);
  }

  status(jobId: string) {
    return this.publicState(this.requireJob(jobId));
  }

  cancel(jobId: string) {
    const job = this.requireJob(jobId);
    if (job.status !== "running") {
      return { ...this.publicState(job), accepted: false };
    }
    if (job.phase === "committing") {
      return { ...this.publicState(job), accepted: false };
    }
    job.cancelled = true;
    job.status = "cancelled";
    this.runningByInput.delete(job.key);
    return { ...this.publicState(job), accepted: true };
  }

  private inputKey(input: unknown) {
    try {
      return JSON.stringify(input);
    } catch {
      throw new ProtocolError(
        "invalid_params",
        "organization input is invalid",
      );
    }
  }

  private requireJob(jobId: string) {
    if (!/^org_[A-Za-z0-9_-]{1,124}$/.test(jobId)) {
      throw new ProtocolError(
        "organization_job_not_found",
        "organization job not found",
      );
    }
    const job = this.jobs.get(jobId);
    if (!job) {
      throw new ProtocolError(
        "organization_job_not_found",
        "organization job not found",
      );
    }
    return job;
  }

  private publicState(job: OrganizationJob) {
    return {
      job_id: job.jobId,
      status: job.status,
      ...(job.result === undefined ? {} : { result: job.result }),
      ...(job.error ? { error: job.error } : {}),
    };
  }

  private checkActive(job: OrganizationJob) {
    if (job.cancelled || job.status === "cancelled") {
      throw new OrganizationCancelled();
    }
  }

  private async run(job: OrganizationJob, input: unknown) {
    try {
      const result = await runOrganizationWorkflow(
        input,
        job.jobId,
        {
          invoke: (method, params, id) =>
            this.options.invokeEngine(method, params, id),
        },
        (value) =>
          this.options.providerHost!.organize(parseOrganizerInput(value)),
        {
          checkActive: () => this.checkActive(job),
          beforeCommit: () => {
            this.checkActive(job);
            job.phase = "committing";
          },
        },
      );
      if (job.status !== "cancelled") {
        job.status = "completed";
        job.result = result;
      }
    } catch (error) {
      if (error instanceof OrganizationCancelled) {
        job.status = "cancelled";
      } else {
        const failure =
          error instanceof ProtocolError
            ? error
            : new ProtocolError(
                "organization_failed",
                error instanceof Error ? error.message : "organization failed",
              );
        job.status = "failed";
        job.error = {
          code: failure.code,
          message: failure.message,
        };
      }
    } finally {
      this.runningByInput.delete(job.key);
    }
  }

  private prune() {
    if (this.jobs.size < MAX_RETAINED_JOBS) return;
    for (const [jobId, job] of this.jobs) {
      if (job.status !== "running") {
        this.jobs.delete(jobId);
      }
      if (this.jobs.size < MAX_RETAINED_JOBS) return;
    }
  }
}
