// 会话优化在详情视图里的展示状态:候选按 locator 映射到轮次、接受后的乐观
// 展示、轮次多选(Shift 范围)、diff 间跳转与 ⌘⏎/⌘⌫ 快捷键。
// 编排(headless Agent、批次写回)在 useSessionOptimization;这里只管视图。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

export function useOptimizationView({
  optimization,
  rounds,
  data,
  metaId,
  canRewrite,
  scrollRef,
}) {
  const optActive = Boolean(optimization?.available && canRewrite);
  const candidates = useMemo(
    () => optimization?.candidates || [],
    [optimization?.candidates],
  );
  const candidateByLocator = useMemo(() => {
    const map = new Map();
    for (const item of candidates) map.set(item.locator, item);
    return map;
  }, [candidates]);
  const pendingTurns = useMemo(
    () => rounds.filter(r => r.locator && candidateByLocator.has(r.locator))
      .map(r => r.n),
    [rounds, candidateByLocator],
  );

  // 接受后气泡立刻显示新文,等批次写回 + 详情刷新后由真实数据接管
  const [acceptedTexts, setAcceptedTexts] = useState({});
  useEffect(() => { setAcceptedTexts({}); }, [data]);
  const resolveCandidate = useCallback((candidate, accept) => {
    if (accept) {
      setAcceptedTexts(prev =>
        ({ ...prev, [candidate.locator]: candidate.text }));
    }
    optimization?.resolve(candidate.locator, accept);
  }, [optimization]);
  const acceptAllCandidates = useCallback(() => {
    setAcceptedTexts(prev => {
      const next = { ...prev };
      for (const item of candidates) next[item.locator] = item.text;
      return next;
    });
    optimization?.acceptAll();
  }, [optimization, candidates]);

  // 多选轮次:Shift+点击选连续范围;开始优化或切换会话时清空
  const [selectedTurns, setSelectedTurns] = useState([]);
  const lastPickRef = useRef(null);
  useEffect(() => { setSelectedTurns([]); }, [metaId]);
  const selectableTurns = useMemo(
    () => rounds.filter(r => r.locator).map(r => r.n),
    [rounds],
  );
  const toggleTurn = useCallback((turn, shiftKey) => {
    setSelectedTurns(previous => {
      const picked = new Set(previous);
      if (shiftKey && lastPickRef.current !== null) {
        const pool = selectableTurns;
        const from = pool.indexOf(lastPickRef.current);
        const to = pool.indexOf(turn);
        if (from >= 0 && to >= 0) {
          for (let i = Math.min(from, to); i <= Math.max(from, to); i++) {
            picked.add(pool[i]);
          }
        }
      } else if (picked.has(turn)) picked.delete(turn);
      else picked.add(turn);
      lastPickRef.current = turn;
      return [...picked].sort((a, b) => a - b);
    });
  }, [selectableTurns]);
  const startOptimization = useCallback(turns => {
    setSelectedTurns([]);
    optimization?.start(turns);
  }, [optimization]);

  // diff 间跳转:悬浮栏 ↑↓ 与缩略指示条共用
  const navIndexRef = useRef(-1);
  const jumpToTurn = useCallback(turn => {
    scrollRef.current
      ?.querySelector(`[data-round="${turn}"]`)
      ?.scrollIntoView({ behavior: "smooth", block: "center" });
  }, [scrollRef]);
  const navDiff = useCallback(direction => {
    if (!pendingTurns.length) return;
    navIndexRef.current =
      (navIndexRef.current + direction + pendingTurns.length)
      % pendingTurns.length;
    jumpToTurn(pendingTurns[navIndexRef.current]);
  }, [pendingTurns, jumpToTurn]);
  // 出候选后把视口带到第一处 diff
  useEffect(() => {
    if (optimization?.status === "reviewing" && pendingTurns.length) {
      navIndexRef.current = 0;
      jumpToTurn(pendingTurns[0]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [optimization?.status]);

  // ⌘⏎ 全部接受 / ⌘⌫ 全部拒绝
  useEffect(() => {
    if (!candidates.length) return;
    const onKey = event => {
      if (!(event.metaKey || event.ctrlKey)) return;
      if (event.key === "Enter") {
        event.preventDefault();
        acceptAllCandidates();
      } else if (event.key === "Backspace") {
        event.preventDefault();
        optimization?.rejectAll();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [candidates.length, acceptAllCandidates, optimization]);

  // 每轮传给 SessionRound 的优化相关 props
  const roundProps = useCallback(r => ({
    onOptimize: optActive && optimization.status !== "running"
      ? () => startOptimization([r.n])
      : undefined,
    optCandidate: r.locator ? candidateByLocator.get(r.locator) : undefined,
    onOptResolve: resolveCandidate,
    optAcceptedText: r.locator ? acceptedTexts[r.locator] : undefined,
    optSelectable: optActive && Boolean(r.locator) && !candidates.length,
    optSelecting: selectedTurns.length > 0,
    optSelected: selectedTurns.includes(r.n),
    onOptToggleSelect: shiftKey => toggleTurn(r.n, shiftKey),
  }), [optActive, optimization, startOptimization, candidateByLocator,
    resolveCandidate, acceptedTexts, candidates.length, selectedTurns,
    toggleTurn]);

  return {
    optActive,
    candidates,
    pendingTurns,
    acceptAllCandidates,
    selectedTurns,
    setSelectedTurns,
    startOptimization,
    jumpToTurn,
    navDiff,
    roundProps,
  };
}
