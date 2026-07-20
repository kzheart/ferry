import { probeFailed } from "../../api/contract/events.js";

export function histStatus(history) {
  if (history.rolled_back) return "已回滚";
  if (probeFailed(history.probe)) return "失败";
  if (history.dry_run) return "预演";
  if (history.session_id) return "成功";
  return "失败";
}
