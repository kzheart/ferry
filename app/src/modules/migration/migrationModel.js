import { sessionIdentity } from "../browser/public.js";

// 会话的迁入来源:在迁移记录里找到以这条会话为产物的那一条(多次命中取最近)。
// 迁移历史没有独立页面,出处直接落在会话详情的元信息里。
export function migrationOriginFor(historyRows, session) {
  const key = sessionIdentity(session);
  if (!key) return null;
  return (historyRows || []).reduce((latest, history) => {
    // 已回滚的迁移在目标工具里没有留下产物,不构成出处
    if (!history?.session_id || history.rolled_back) return latest;
    if (sessionIdentity({ tool: history.dst, id: history.session_id }) !== key) return latest;
    if (!latest || String(history.time || "") > String(latest.time || "")) return history;
    return latest;
  }, null);
}
