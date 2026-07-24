// 当前会话的待应用编辑。
//
// 同一份状态被两处渲染:资料库详情区,以及 Ask Ferry 卡片的就地预览浮层。两条
// 路径各自要穿过若干只转交不使用的层(WorkspaceRouter / SessionPeekSheet),
// 于是同一个字段要在四五个签名里各写一遍。
//
// 注意 value 必须是 memo 化的:详情区是 memo 组件,靠"编辑态没变就不重渲染"
// 躲开侧边栏交互(展开分组、多选、悬停)引起的整条时间线重绘。每次渲染都新建
// 一个 value 会把这层优化全部作废。
import { createContext, useContext } from "react";

const SessionEditingContext = createContext(null);

export function SessionEditingProvider({ value, children }) {
  return (
    <SessionEditingContext.Provider value={value}>
      {children}
    </SessionEditingContext.Provider>
  );
}

export function useSessionEditingSurface() {
  const value = useContext(SessionEditingContext);
  if (!value) {
    throw new Error("useSessionEditingSurface 必须在 SessionEditingProvider 内使用");
  }
  return value;
}
