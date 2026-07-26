// 破坏性操作与 Ask Ferry 域的壳状态:整理、迁移、待应用编辑、浮动面板、
// Agent 会话重命名。
//
// 与 browserState 同源的动机:这些字段原先经 workspaceOverlayProps 组装后层层
// 转交,中间层只转发不使用。按域拆开之后,改一个弹层的入参不再牵动整条链路。
//
// value 必须 memo 化,理由同 browserState:下游有 memo 组件靠引用相等跳过重渲染。
import { createContext, useContext } from "react";

const OperationsStateContext = createContext(null);

export function OperationsStateProvider({ value, children }) {
  return (
    <OperationsStateContext.Provider value={value}>
      {children}
    </OperationsStateContext.Provider>
  );
}

export function useOperationsState() {
  const value = useContext(OperationsStateContext);
  if (!value) {
    throw new Error("useOperationsState 必须在 OperationsStateProvider 内使用");
  }
  return value;
}
