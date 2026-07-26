// 应用外壳自身的状态:设置弹层、更新器、引导、导航轨提示、全局 Toast。
//
// 这一域和具体业务无关,是"应用这个壳"的状态。与 browserState /
// operationsState 一起取代了原先 workspaceOverlayProps 的 80 参数转交。
//
// value 必须 memo 化,理由同另外两个域。
import { createContext, useContext } from "react";

const AppChromeContext = createContext(null);

export function AppChromeProvider({ value, children }) {
  return (
    <AppChromeContext.Provider value={value}>
      {children}
    </AppChromeContext.Provider>
  );
}

export function useAppChrome() {
  const value = useContext(AppChromeContext);
  if (!value) {
    throw new Error("useAppChrome 必须在 AppChromeProvider 内使用");
  }
  return value;
}
