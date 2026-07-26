// Ask Ferry 状态中枢:订阅 ferry-runtime-event,维护会话列表与每会话时间线,
// 打开会话时用 events.replay 回放(seq 去重保证与实时流合并一致)
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  onRuntimeEvent,
  operationApply,
  pickSkillDirectory,
  runtime,
  shellApply,
} from "../../platform/desktop/client.js";
import { applyEvent, emptyLog, operationKey, patchApproval }
  from "./agentChatModel.js";

const MODE_KEY = "ferry-askferry-mode";
const RUN_TYPES = new Set(["run.started", "run.completed", "run.failed",
  "run.cancelled", "run.interrupted"]);
// 后台会话的提醒等级:待审批最紧急,其次是失败,最后是「跑完了还没看」。
// 低等级不覆盖高等级,避免一条 run.completed 把待审批的提示冲掉。
const ATTENTION_RANK = { approval: 3, error: 2, unread: 1 };
const attentionOf = type => {
  if (type === "tool.request" || type === "operation.proposed") return "approval";
  if (type === "run.failed") return "error";
  if (type === "run.completed") return "unread";
  return null;
};

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
  // 已导入技能与外部候选分成两份状态:候选每次进技能页才扫,不跟着 roles 一起刷
  const [skills, setSkills] = useState(
    { skills: [], global: [], scan_sources: [] });
  const [skillCandidates, setSkillCandidates] = useState(
    { candidates: [], sources: [] });
  const [selectedRoleId, setSelectedRoleId] = useState("default");
  const [lastError, setLastError] = useState(null);
  const [mutationVersion, setMutationVersion] = useState(0);

  const logsRef = useRef(logs); logsRef.current = logs;
  const activeRef = useRef(activeId); activeRef.current = activeId;
  const modeRef = useRef(mode); modeRef.current = mode;
  // 回放期间到达的实时事件先入队,回放完成后按 seq 合并,避免丢历史
  const loadingRef = useRef(new Map());
  const refreshRef = useRef(() => {});

  const setMode = useCallback(m => {
    setModeState(m);
    localStorage.setItem(MODE_KEY, m);
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
    const opId = operationKey(item.operation);
    if (!opId) return;
    mutateLog(sessionId, log => patchApproval(log, opId, { status: "applying", auto }));
    try {
      // shl_ 是 bash 提案:命令在 Rust 侧执行,不走 Engine 的 operation 状态机
      const result = await (opId.startsWith("shl_")
        ? shellApply(opId)
        : operationApply(opId));
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
    onRuntimeEvent(ev => {
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
      // 标题由 runtime 在首轮结束后生成;auto=false 说明是用户手动改名,顺带锁住
      if (ev.type === "session.renamed") {
        const p = ev.payload || {};
        if (p.title) {
          setSessions(list => list.map(s => s.session_id === sid
            ? { ...s, title: p.title, title_locked: p.auto ? s.title_locked : true }
            : s));
        }
        return;
      }
      const isRun = RUN_TYPES.has(ev.type);
      const attention = attentionOf(ev.type);
      if (isRun || attention) {
        setSessions(list => list.map(s => {
          if (s.session_id !== sid) return s;
          const next = { ...s };
          if (isRun) {
            next.status = ev.type === "run.started" ? "running" : "idle";
            next.updated_at = ev.timestamp || s.updated_at;
          }
          // 用户正在看这个会话(或它刚开始新一轮),徽标没有意义
          if (ev.type === "run.started" || sid === activeRef.current) next.attention = null;
          else if (attention &&
            ATTENTION_RANK[attention] >= (ATTENTION_RANK[s.attention] || 0)) {
            next.attention = attention;
          }
          return next;
        }));
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
        runtime("health"), runtime("sessions.list"),
        runtime("roles.list")]);
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
    if (id) {
      setSessions(list => list.some(s => s.session_id === id && s.attention)
        ? list.map(s => s.session_id === id ? { ...s, attention: null } : s)
        : list);
    }
    if (!id || logsRef.current[id] || loadingRef.current.has(id)) return;
    loadingRef.current.set(id, []);
    try {
      const events = await runtime("events.replay", {
        session_id: id,
        after_seq: 0,
      });
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
  // 手动改名会把 title_locked 置上(runtime 回传),自动命名从此不再覆盖
  const rename = useCallback(async (id, title) => {
    const state = await runtime("session.rename", { session_id: id, title });
    setSessions(list => list.map(s => s.session_id === id
      ? { ...s, ...state, title_locked: true } : s));
    return state;
  }, []);

  const pin = useCallback(async (id, pinned) => {
    const state = await runtime("session.pin", { session_id: id, pinned });
    setSessions(list => list.map(s => s.session_id === id ? { ...s, ...state } : s));
    return state;
  }, []);

  const deleteSession = useCallback(async id => {
    await runtime("session.delete", { session_id: id });
    setSessions(list => list.filter(s => s.session_id !== id));
    setLogs(prev => { const { [id]: _deleted, ...next } = prev; return next; });
    if (activeRef.current === id) setActiveId(null);
  }, []);

  const send = useCallback(async (text, displayText = text) => {
    let sid = activeRef.current;
    if (!sid) {
      const state = await runtime("session.create", {
        role_id: selectedRoleId,
      });
      sid = state.session_id;
      const log = { ...emptyLog(), provider: state.provider_id, model: state.model_id };
      setLogs(prev => ({ ...prev, [sid]: log }));
      setSessions(list => [{ ...state, updated_at: new Date().toISOString() }, ...list]);
      setActiveId(sid);
    }
    const running = logsRef.current[sid]?.status === "running";
    // 标题不在这里定:runtime 会在首轮跑完后用模型生成,再经 session.renamed 推回来
    await runtime(running ? "follow_up" : "prompt", {
      session_id: sid,
      text,
      display_text: displayText,
      auto_apply: modeRef.current === "auto",
    });
    return sid;
  }, [selectedRoleId]);

  const reloadRoles = useCallback(async () => {
    const list = await runtime("roles.list");
    setRoles(list || []);
    if (!(list || []).some(role => role.id === selectedRoleId)) {
      setSelectedRoleId("default");
    }
    return list || [];
  }, [selectedRoleId]);
  const createRole = useCallback(async role => {
    const result = await runtime("role.create", { role });
    await reloadRoles();
    return result;
  }, [reloadRoles]);
  const updateRole = useCallback(async role => {
    const result = await runtime("role.update", { role_id: role.id, role });
    await reloadRoles();
    return result;
  }, [reloadRoles]);
  // 内置角色改坏了可以回到出厂设置:运行时丢掉覆盖层,自定义角色不受影响
  const resetRole = useCallback(async roleId => {
    const result = await runtime("role.reset", { role_id: roleId });
    await reloadRoles();
    return result;
  }, [reloadRoles]);
  const copyRole = useCallback(async (sourceRoleId, roleId, name) => {
    const result = await runtime("role.copy", {
      source_role_id: sourceRoleId, role_id: roleId, name,
    });
    await reloadRoles();
    return result;
  }, [reloadRoles]);
  const deleteRole = useCallback(async roleId => {
    const result = await runtime("role.delete", { role_id: roleId });
    await reloadRoles();
    return result;
  }, [reloadRoles]);

  const reloadSkills = useCallback(async () => {
    const listing = await runtime("skills.list").catch(() => null);
    const next = listing || { skills: [], global: [], scan_sources: [] };
    setSkills(next);
    return next;
  }, []);
  const loadSkillCandidates = useCallback(async () => {
    const found = await runtime("skills.candidates").catch(() => null);
    const next = found || { candidates: [], sources: [] };
    setSkillCandidates(next);
    return next;
  }, []);
  const importSkill = useCallback(async params => {
    const result = await runtime("skill.import", params);
    await reloadSkills();
    await loadSkillCandidates();
    return result;
  }, [reloadSkills, loadSkillCandidates]);
  const deleteSkill = useCallback(async skillId => {
    const result = await runtime("skill.delete", { skill_id: skillId });
    await reloadSkills();
    await loadSkillCandidates();
    return result;
  }, [reloadSkills, loadSkillCandidates]);
  const setGlobalSkills = useCallback(async skillIds => {
    const result = await runtime("skills.global.set", { skill_ids: skillIds });
    await reloadSkills();
    return result;
  }, [reloadSkills]);
  const addSkillSource = useCallback(async () => {
    const path = await pickSkillDirectory();
    if (!path) return null;
    const result = await runtime("skill.source.add", { path });
    await reloadSkills();
    await loadSkillCandidates();
    return result;
  }, [reloadSkills, loadSkillCandidates]);
  const removeSkillSource = useCallback(async sourceId => {
    const result = await runtime("skill.source.remove", { source_id: sourceId });
    await reloadSkills();
    await loadSkillCandidates();
    return result;
  }, [reloadSkills, loadSkillCandidates]);
  /** 从文件夹直接导入:路径由系统对话框产生,webview 不能凭空指定。 */
  const importSkillFolder = useCallback(async () => {
    const path = await pickSkillDirectory();
    if (!path) return null;
    return importSkill({ path });
  }, [importSkill]);
  const readSkill = useCallback(
    skillId => runtime("skill.read", { skill_id: skillId }), []);

  const steer = useCallback((text, displayText = text) =>
    runtime("steer", { session_id: activeRef.current, text,
      display_text: displayText, auto_apply: modeRef.current === "auto" }), []);
  const abort = useCallback(() =>
    runtime("abort", { session_id: activeRef.current }).catch(() => {}), []);

  // 模型选择器的候选:已启用且已配置凭据的 Provider 下用户勾选可见的模型
  const loadModels = useCallback(async () => {
    if (!available) return [];
    const list = await runtime("models.enabled").catch(() => []);
    setModels(list || []);
    return list || [];
  }, [available]);
  useEffect(() => { loadModels(); }, [loadModels]);

  // 切模型/切推理强度:默认与当前对话一起改,新开对话也就沿用同一个选择
  const selectModel = useCallback(async (providerId, modelId, thinking) => {
    const params = { provider_id: providerId, model_id: modelId };
    if (thinking) params.thinking = thinking;
    const result = await runtime("model.select", params);
    setHealth(h => h ? { ...h, provider: providerId, model: modelId,
      thinking: thinking || "off" } : h);
    if (activeRef.current) {
      await runtime("model.select", {
        ...params,
        session_id: activeRef.current,
      })
        .catch(() => {});
    }
    return result;
  }, []);

  // ----- Provider 登录(OAuth / 交互式) -----
  const startLogin = useCallback(async (providerId, authType) => {
    const r = await runtime("auth.login.start",
      { provider_id: providerId, auth_type: authType });
    setAuth({ loginId: r.login_id, providerId, authType,
      prompts: [], notices: [], status: "running" });
    return r;
  }, []);
  const respondLogin = useCallback(async (promptId, value) => {
    const loginId = auth?.loginId;
    if (!loginId) return;
    await runtime("auth.login.respond",
      { login_id: loginId, prompt_id: promptId, value });
    setAuth(prev => prev
      ? { ...prev, prompts: prev.prompts.filter(p => p.promptId !== promptId) } : prev);
  }, [auth?.loginId]);
  const cancelLogin = useCallback(() => {
    if (auth?.loginId) runtime("auth.login.cancel",
      { login_id: auth.loginId }).catch(() => {});
    setAuth(null);
  }, [auth?.loginId]);
  const clearAuth = useCallback(() => setAuth(null), []);

  const clearError = useCallback(() => setLastError(null), []);

  const activeLog = activeId ? logs[activeId] : null;
  // 这个对象经 context 发给所有消费者:每次渲染新建一个,等于让根组件的任何
  // state 变化都把整棵 Ask Ferry 子树重渲染一遍。写法同 SessionEditingProvider。
  return useMemo(() => ({
    available, health, sessions, activeId, activeLog, mode, auth, models, mutationVersion,
    roles, selectedRoleId, setSelectedRoleId,
    skills, skillCandidates,
    lastError, clearError, reportError: setLastError,
    refresh, openSession, newChat, send, steer, abort, setMode, rename, pin, deleteSession,
    approve, dismiss, selectModel, loadModels,
    reloadRoles, createRole, updateRole, resetRole, copyRole, deleteRole,
    reloadSkills, loadSkillCandidates, importSkill, importSkillFolder, deleteSkill,
    setGlobalSkills, addSkillSource, removeSkillSource, readSkill,
    startLogin, respondLogin, cancelLogin, clearAuth,
  }), [
    available, health, sessions, activeId, activeLog, mode, auth, models, mutationVersion,
    roles, selectedRoleId, setSelectedRoleId,
    skills, skillCandidates,
    lastError, clearError,
    refresh, openSession, newChat, send, steer, abort, setMode, rename, pin, deleteSession,
    approve, dismiss, selectModel, loadModels,
    reloadRoles, createRole, updateRole, resetRole, copyRole, deleteRole,
    reloadSkills, loadSkillCandidates, importSkill, importSkillFolder, deleteSkill,
    setGlobalSkills, addSkillSource, removeSkillSource, readSkill,
    startLogin, respondLogin, cancelLogin, clearAuth,
  ]);
}
