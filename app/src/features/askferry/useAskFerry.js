// Ask Ferry 状态中枢:订阅 ferry-agent-event,维护会话列表与每会话时间线,
// 打开会话时用 events.replay 回放(seq 去重保证与实时流合并一致)
import { useCallback, useEffect, useRef, useState } from "react";
import { agentCommand, onAgentEvent,
  operationPlanApply } from "../../api/agent/agentClient.js";
import { applyEvent, emptyLog, operationKey, patchApproval }
  from "../../domain/agent/agentChatModel.js";

const MODE_KEY = "ferry-askferry-mode";
const RUN_TYPES = new Set(["run.started", "run.completed", "run.failed",
  "run.cancelled", "run.interrupted"]);

export function useAskFerry() {
  const available = true;
  const [sessions, setSessions] = useState([]);
  const [activeId, setActiveId] = useState(null);
  const [logs, setLogs] = useState({});
  const [health, setHealth] = useState(null);
  const [mode, setModeState] = useState(() => localStorage.getItem(MODE_KEY) || "auto");
  const [auth, setAuth] = useState(null);
  const [models, setModels] = useState([]);
  const [roles, setRoles] = useState([]);
  const [selectedRoleId, setSelectedRoleId] = useState("default");
  const [lastError, setLastError] = useState(null);
  const [mutationVersion, setMutationVersion] = useState(0);

  const logsRef = useRef(logs); logsRef.current = logs;
  const activeRef = useRef(activeId); activeRef.current = activeId;
  const modeRef = useRef(mode); modeRef.current = mode;
  // 回放期间到达的实时事件先入队,回放完成后按 seq 合并,避免丢历史
  const loadingRef = useRef(new Map());
  const refreshRef = useRef(() => {});

  const setMode = m => { setModeState(m); localStorage.setItem(MODE_KEY, m); };

  const mutateLog = useCallback((id, fn) => {
    setLogs(prev => {
      const log = prev[id];
      if (!log) return prev;
      const next = fn(log);
      return next === log ? prev : { ...prev, [id]: next };
    });
  }, []);

  const approve = useCallback(async (sessionId, item, auto = false) => {
    const opId = operationKey(item.operation);
    if (!opId) return;
    mutateLog(sessionId, log => patchApproval(log, opId, { status: "applying", auto }));
    try {
      const result = await operationPlanApply(opId);
      mutateLog(sessionId, log => patchApproval(log, opId, { status: "applied", result, auto }));
      setMutationVersion(value => value + 1);
      return result;
    } catch (error) {
      mutateLog(sessionId, log =>
        patchApproval(log, opId, { status: "failed", error: String(error), auto }));
    }
  }, [mutateLog]);
  const dismiss = useCallback((sessionId, item) => {
    const opId = operationKey(item.operation);
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
      if (ev.type === "operation.applied") setMutationVersion(value => value + 1);
    }).then(u => { un = u; });
    return () => un?.();
  }, [available]);

  // ----- 启动:健康检查 + 会话列表 -----
  const refresh = useCallback(async () => {
    if (!available) return;
    try {
      const [h, list, roleList] = await Promise.all([
        agentCommand("health"), agentCommand("sessions.list"),
        agentCommand("roles.list")]);
      setHealth(h);
      setSessions(list || []);
      setRoles(roleList || []);
      if (!(roleList || []).some(role => role.id === selectedRoleId)) {
        setSelectedRoleId("default");
      }
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
    } catch (error) {
      loadingRef.current.delete(id);
      setLastError(error);
    }
  }, []);

  const newChat = useCallback(() => setActiveId(null), []);

  // ----- 发送:无会话则先创建;运行中改走 follow_up -----
  const rename = useCallback(async (id, title) => {
    const state = await agentCommand("session.rename", { session_id: id, title });
    setSessions(list => list.map(s => s.session_id === id ? { ...s, ...state } : s));
    return state;
  }, []);

  const pin = useCallback(async (id, pinned) => {
    const state = await agentCommand("session.pin", { session_id: id, pinned });
    setSessions(list => list.map(s => s.session_id === id ? { ...s, ...state } : s));
    return state;
  }, []);

  const deleteSession = useCallback(async id => {
    await agentCommand("session.delete", { session_id: id });
    setSessions(list => list.filter(s => s.session_id !== id));
    setLogs(prev => { const { [id]: _deleted, ...next } = prev; return next; });
    if (activeRef.current === id) setActiveId(null);
  }, []);

  const send = useCallback(async (text, displayText = text) => {
    let sid = activeRef.current;
    if (!sid) {
      const state = await agentCommand("session.create", {
        role_id: selectedRoleId,
      });
      sid = state.session_id;
      const log = { ...emptyLog(), provider: state.provider_id, model: state.model_id };
      setLogs(prev => ({ ...prev, [sid]: log }));
      setSessions(list => [{ ...state, updated_at: new Date().toISOString() }, ...list]);
      setActiveId(sid);
    }
    const running = logsRef.current[sid]?.status === "running";
    await agentCommand(running ? "follow_up" : "prompt", {
      session_id: sid,
      text,
      display_text: displayText,
      auto_apply: modeRef.current === "auto",
    });
    if (!running && !sessions.find(s => s.session_id === sid)?.title) {
      const title = displayText.split("\n")[0].trim().slice(0, 200);
      if (title) rename(sid, title).catch(() => {});
    }
    return sid;
  }, [rename, selectedRoleId, sessions]);

  const reloadRoles = useCallback(async () => {
    const list = await agentCommand("roles.list");
    setRoles(list || []);
    if (!(list || []).some(role => role.id === selectedRoleId)) {
      setSelectedRoleId("default");
    }
    return list || [];
  }, [selectedRoleId]);
  const createRole = useCallback(async role => {
    const result = await agentCommand("role.create", { role });
    await reloadRoles();
    return result;
  }, [reloadRoles]);
  const updateRole = useCallback(async role => {
    const result = await agentCommand("role.update", { role_id: role.id, role });
    await reloadRoles();
    return result;
  }, [reloadRoles]);
  const copyRole = useCallback(async (sourceRoleId, roleId, name) => {
    const result = await agentCommand("role.copy", {
      source_role_id: sourceRoleId, role_id: roleId, name,
    });
    await reloadRoles();
    return result;
  }, [reloadRoles]);
  const deleteRole = useCallback(async roleId => {
    const result = await agentCommand("role.delete", { role_id: roleId });
    await reloadRoles();
    return result;
  }, [reloadRoles]);

  const steer = useCallback((text, displayText = text) =>
    agentCommand("steer", { session_id: activeRef.current, text,
      display_text: displayText, auto_apply: modeRef.current === "auto" }), []);
  const abort = useCallback(() =>
    agentCommand("abort", { session_id: activeRef.current }).catch(() => {}), []);

  // 模型选择器的候选:已启用且已配置凭据的 Provider 下用户勾选可见的模型
  const loadModels = useCallback(async () => {
    if (!available) return [];
    const list = await agentCommand("models.enabled").catch(() => []);
    setModels(list || []);
    return list || [];
  }, [available]);
  useEffect(() => { loadModels(); }, [loadModels]);

  // 切模型/切推理强度:默认与当前对话一起改,新开对话也就沿用同一个选择
  const selectModel = useCallback(async (providerId, modelId, thinking) => {
    const params = { provider_id: providerId, model_id: modelId };
    if (thinking) params.thinking = thinking;
    const result = await agentCommand("model.select", params);
    setHealth(h => h ? { ...h, provider: providerId, model: modelId,
      thinking: thinking || "off" } : h);
    if (activeRef.current) {
      await agentCommand("model.select", { ...params, session_id: activeRef.current })
        .catch(() => {});
    }
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
    available, health, sessions, activeId, activeLog, mode, auth, models, mutationVersion,
    roles, selectedRoleId, setSelectedRoleId,
    lastError, clearError: () => setLastError(null), reportError: setLastError,
    refresh, openSession, newChat, send, steer, abort, setMode, rename, pin, deleteSession,
    approve, dismiss, selectModel, loadModels,
    reloadRoles, createRole, updateRole, copyRole, deleteRole,
    startLogin, respondLogin, cancelLogin, clearAuth,
  };
}
