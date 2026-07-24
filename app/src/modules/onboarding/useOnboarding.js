import { useEffect, useRef, useState } from "react";

const FIRST_RUN_KEY = "ferry-first-done";
const GUIDE_SEEN_KEY = "ferry-guide-seen";

export function initialWorkspace() {
  return localStorage.getItem(FIRST_RUN_KEY) ? "overview" : "firstrun";
}

export function useOnboarding({
  setView,
  closeSettings,
  closeMigration,
  scan,
}) {
  const [step, setStep] = useState(0);
  const [seen, setSeen] = useState(
    () => localStorage.getItem(GUIDE_SEEN_KEY) === "1",
  );
  const timer = useRef(null);
  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);

  const openGuide = () => {
    setView("library");
    closeSettings();
    closeMigration();
    setStep(1);
  };

  const finishGuide = () => {
    setStep(0);
    setSeen(true);
    localStorage.setItem(GUIDE_SEEN_KEY, "1");
  };

  const completeFirstRun = () => {
    localStorage.setItem(FIRST_RUN_KEY, "1");
    setView("library");
    scan();
    if (!seen) timer.current = setTimeout(() => setStep(1), 300);
  };

  return {
    step,
    seen,
    setStep,
    openGuide,
    finishGuide,
    completeFirstRun,
  };
}
