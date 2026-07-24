export const STATUS_CODE = {
  success: "status.success",
  failed: "status.failed",
  rolledBack: "status.rolled_back",
};

function probeFailed(probe) {
  return !!probe && (probe.status === "failed" || probe.ok === false);
}

export function histStatus(history) {
  if (history.rolled_back) return STATUS_CODE.rolledBack;
  if (probeFailed(history.probe)) return STATUS_CODE.failed;
  if (history.session_id) return STATUS_CODE.success;
  return STATUS_CODE.failed;
}
