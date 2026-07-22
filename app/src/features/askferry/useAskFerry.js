// Ask Ferry 状态中枢:订阅 ferry-agent-event,维护会话列表与每会话时间线,
// 打开会话时用 events.replay 回放(seq 去重保证与实时流合并一致)
import { useCallback, useEffect, useRef, useState } from "react";
import { agentAvailable, agentCommand, onAgentEvent,
  operationApproveAndApply } from "../../api/agent/agentClient.js";
import { applyEvent, emptyLog, patchApproval, titleOf }
  from "../../domain/agent/agentChatModel.js";

const MODE_KEY = "ferry-askferry-mode";
const TITLES_KEY = "ferry-agent-titles";
const RUN_TYPES = new Set(["run.started", "run.completed", "run.failed",
  "run.cancelled", "run.interrupted"]);
// 自动模式只放行迁移与元数据;编辑另存副本仍要求人工确认
const AUTO_TOOLS = new Set(["ferry_propose_migration", "ferry_propose_metadata_change"]);

const readTitles = () => {
  try { return JSON.parse(localStorage.getItem(TITLES_KEY) || "{}"); }
  catch { return {}; }
};

export function useAskFerry() {
  const available = agentAvailable();
  const [sessions, setSessions] = useState([]);
  const [activeId, setActiveId] = useState(null);
  const [logs, setLogs] = useState({});
  const [titles, setTitles] = useState(readTitles);
  const [health, setHealth] = useState(null);
  const [mode, setModeState] = useState(() => localStorage.getItem(MODE_KEY) || "manual");
  const [auth, setAuth] = useState(null);
  const [lastError, setLastError] = useState(null);

  const logsRef = useRef(logs); logsRef.current = logs;
  const activeRef = useRef(activeId); activeRef.current = activeId;
  const modeRef = useRef(mode); modeRef.current = mode;
  // 回放期间到达的实时事件先入队,回放完成后按 seq 合并,避免丢历史
  const loadingRef = useRef(new Map());
  const refreshRef = useRef(() => {});

  const setMode = m => { setModeState(m); localStorage.setItem(MODE_KEY, m); };

  const rememberTitle = useCallback((id, title) => {
    if (!title) return;
    setTitles(prev => {
      if (prev[id] === title) return prev;
      const next = { ...prev, [id]: title };
      try { localStorage.setItem(TITLES_KEY, JSON.stringify(next)); } catch {}
      return next;
    });
  }, []);

  const mutateLog = useCallback((id, fn) => {
    setLogs(prev => {
      const log = prev[id];
      if (!log) return prev;
      const next = fn(log);
      return next === log ? prev : { ...prev, [id]: next };
    });
  }, []);

  const approve = useCallback(async (sessionId, item, auto = false) => {
    const opId = item.operation?.operation_id;
    if (!opId) return;
    mutateLog(sessionId, log => patchApproval(log, opId, { status: "applying", auto }));
    try {
      const result = await operationApproveAndApply(opId, item.runId || "");
      mutateLog(sessionId, log => patchApproval(log, opId, { status: "applied", result, auto }));
    } catch (error) {
      mutateLog(sessionId, log =>
        patchApproval(log, opId, { status: "failed", error: String(error), auto }));
    }
  }, [mutateLog]);
  const approveRef = useRef(approve); approveRef.current = approve;

  const dismiss = useCallback((sessionId, item) => {
    const opId = item.operation?.operation_id;
    if (opId) mutateLog(sessionId, log => patchApproval(log, opId, { status: "dismissed" }));
  }, [mutateLog]);

  // ----- 事件订阅 -----
  useEffect(() => {
    if (!available) return;
    let un;
    onAgentEvent(ev => {
      if (!ev || typeof ev !== "object") return;
      if (ev.type === "runtime.disconnected") {
        // 进程退出:运行中的时间线就地标记 interrupted,不自动重放
        setLogs(prev => {
          const next = { ...prev };
          for (const [id, log] of Object.entries(prev)) {
            if (log.status === "running") {
              next[id] = applyEvent(log, { type: "run.interrupted",
                session_id: id, run_id: log.runId, payload: {} });
            }
          }
          return next;
        });
        setSessions(list => list.map(s =>
          s.status === "running" ? { ...s, status: "idle" } : s));
        // 稍后重新探测:health 请求会让 supervisor 惰性重启 sidecar 并刷新凭据状态
        setTimeout(() => refreshRef.current(), 500);
        return;
      }
      if (ev.session_id === "runtime") {
        setAuth(prev => {
          const p = ev.payload || {};
          if (ev.type === "auth.prompt") {
            if (!prev || prev.loginId !== p.login_id) return prev;
            return { ...prev, prompts: [...prev.prompts,
              { promptId: p.prompt_id, ...p.prompt }] };
          }
          if (ev.type === "auth.event") {
            if (!prev || prev.loginId !== p.login_id) return prev;
            return { ...prev, notices: [...prev.notices, p.event || {}] };
          }
          if (["auth.completed", "auth.failed", "auth.cancelled"].includes(ev.type)) {
            if (!prev || prev.loginId !== p.login_id) return prev;
            return { ...prev, status: ev.type.slice(5), message: p.message, prompts: [] };
          }
          return prev;
        });
        return;
      }
      const sid = ev.session_id;
      if (!sid) return;
      const pending = loadingRef.current.get(sid);
      if (pending) { pending.push(ev); return; }
      if (logsRef.current[sid]) {
        setLogs(prev => prev[sid]
          ? { ...prev, [sid]: applyEvent(prev[sid], ev) } : prev);
      }
      if (RUN_TYPES.has(ev.type)) {
        setSessions(list => list.map(s => s.session_id === sid
          ? { ...s, status: ev.type === "run.started" ? "running" : "idle",
              updated_at: ev.timestamp || s.updated_at }
          : s));
      }
      if (ev.type === "operation.proposed" && modeRef.current === "auto"
          && AUTO_TOOLS.has(ev.payload?.tool)) {
        const item = { operation: ev.payload?.operation || {}, runId: ev.run_id };
        approveRef.current(sid, item, true);
      }
    }).then(u => { un = u; });
    return () => un?.();
  }, [available]);

  // ----- 启动:健康检查 + 会话列表 -----
  const refresh = useCallback(async () => {
    if (!available) return;
    try {
      const [h, list] = await Promise.all([
        agentCommand("health"), agentCommand("sessions.list")]);
      setHealth(h);
      setSessions(list || []);
    } catch (error) {
      setLastError(error);
    }
  }, [available]);
  useEffect(() => { refreshRef.current = refresh; }, [refresh]);
  useEffect(() => { refresh(); }, [refresh]);

  // ----- 打开会话:回放事件 -----
  const openSession = useCallback(async id => {
    setActiveId(id);
    if (!id || logsRef.current[id] || loadingRef.current.has(id)) return;
    loadingRef.current.set(id, []);
    try {
      const events = await agentCommand("events.replay", { session_id: id, after_seq: 0 });
      let log = emptyLog();
      for (const ev of events || []) log = applyEvent(log, ev);
      for (const ev of loadingRef.current.get(id) || []) log = applyEvent(log, ev);
      loadingRef.current.delete(id);
      setLogs(prev => ({ ...prev, [id]: log }));
      rememberTitle(id, titleOf(log));
    } catch (error) {
      loadingRef.current.delete(id);
      setLastError(error);
    }
  }, [rememberTitle]);

  const newChat = useCallback(() => setActiveId(null), []);

  // ----- 发送:无会话则先创建;运行中改走 follow_up -----
  const send = useCallback(async text => {
    let sid = activeRef.current;
    if (!sid) {
      const state = await agentCommand("session.create", {});
      sid = state.session_id;
      const log = { ...emptyLog(), provider: state.provider_id, model: state.model_id };
      setLogs(prev => ({ ...prev, [sid]: log }));
      setSessions(list => [{ ...state, updated_at: new Date().toISOString() }, ...list]);
      setActiveId(sid);
    }
    const running = logsRef.current[sid]?.status === "running";
    await agentCommand(running ? "follow_up" : "prompt", { session_id: sid, text });
    if (!running) rememberTitle(sid, text.split("\n")[0].slice(0, 60));
    return sid;
  }, [rememberTitle]);

  const steer = useCallback(text =>
    agentCommand("steer", { session_id: activeRef.current, text }), []);
  const abort = useCallback(() =>
    agentCommand("abort", { session_id: activeRef.current }).catch(() => {}), []);

  const selectModel = useCallback(async (providerId, modelId, forSession) => {
    const params = { provider_id: providerId, model_id: modelId };
    if (forSession && activeRef.current) params.session_id = activeRef.current;
    const result = await agentCommand("model.select", params);
    if (!forSession) setHealth(h => h ? { ...h, provider: providerId, model: modelId } : h);
    return result;
  }, []);

  // ----- Provider 登录(OAuth / 交互式) -----
  const startLogin = useCallback(async (providerId, authType) => {
    const r = await agentCommand("auth.login.start",
      { provider_id: providerId, auth_type: authType });
    setAuth({ loginId: r.login_id, providerId, authType,
      prompts: [], notices: [], status: "running" });
    return r;
  }, []);
  const respondLogin = useCallback(async (promptId, value) => {
    const loginId = auth?.loginId;
    if (!loginId) return;
    await agentCommand("auth.login.respond",
      { login_id: loginId, prompt_id: promptId, value });
    setAuth(prev => prev
      ? { ...prev, prompts: prev.prompts.filter(p => p.promptId !== promptId) } : prev);
  }, [auth?.loginId]);
  const cancelLogin = useCallback(() => {
    if (auth?.loginId) agentCommand("auth.login.cancel",
      { login_id: auth.loginId }).catch(() => {});
    setAuth(null);
  }, [auth?.loginId]);
  const clearAuth = useCallback(() => setAuth(null), []);

  const activeLog = activeId ? logs[activeId] : null;
  return {
    available, health, sessions, titles, activeId, activeLog, mode, auth,
    lastError, clearError: () => setLastError(null), reportError: setLastError,
    refresh, openSession, newChat, send, steer, abort, setMode,
    approve, dismiss, selectModel,
    startLogin, respondLogin, cancelLogin, clearAuth,
  };
}
