import { useEffect, useRef, useState } from "react";

import { guideSteps } from "./steps.js";

const FIRST_RUN_KEY = "ferry-first-done";
const GUIDE_SEEN_KEY = "ferry-guide-seen";

export function initialWorkspace() {
  return localStorage.getItem(FIRST_RUN_KEY) ? "overview" : "firstrun";
}

export function useOnboarding({
  setView,
  closeSettings,
  closeMigration,
  isFeatureEnabled,
}) {
  const [step, setStep] = useState(0);
  const [seen, setSeen] = useState(
    () => localStorage.getItem(GUIDE_SEEN_KEY) === "1",
  );
  const timer = useRef(null);
  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);

  const steps = guideSteps(isFeatureEnabled);

  // 各步骤分属不同模块,进入某一步时先切到它所在的视图
  const goStep = (next) => {
    const view = steps[next - 1]?.view;
    if (view) setView(view);
    setStep(next);
  };

  const openGuide = () => {
    closeSettings();
    closeMigration();
    goStep(1);
  };

  const finishGuide = () => {
    setStep(0);
    setSeen(true);
    localStorage.setItem(GUIDE_SEEN_KEY, "1");
    setView("library");
  };

  // 扫描已经在向导终点站里跑过了,这里只负责落标记、切视图、接上导览
  const completeFirstRun = () => {
    localStorage.setItem(FIRST_RUN_KEY, "1");
    setView("library");
    if (!seen) timer.current = setTimeout(() => goStep(1), 300);
  };

  return {
    step,
    steps,
    seen,
    goStep,
    openGuide,
    finishGuide,
    completeFirstRun,
  };
}
