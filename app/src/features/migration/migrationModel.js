import { probeFailed } from "../../api/contract/events.js";

export const STATUS_CODE = {
  success: "status.success",
  failed: "status.failed",
  rolledBack: "status.rolled_back",
  dryRun: "status.dry_run",
};

export function histStatus(history) {
  if (history.rolled_back) return STATUS_CODE.rolledBack;
  if (probeFailed(history.probe)) return STATUS_CODE.failed;
  if (history.dry_run) return STATUS_CODE.dryRun;
  if (history.session_id) return STATUS_CODE.success;
  return STATUS_CODE.failed;
}
