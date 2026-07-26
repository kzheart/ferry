// 会话浏览域的壳状态:预览、搜索、右键菜单、删除确认、重命名、标签、筛选。
//
// 这些字段原先由 workspaceOverlayProps.buildOverlayProps 组装成一个 80 参数的
// 大对象,再穿过 AppController -> AppOverlayController / WorkspaceRouter 层层
// 转交;中间层大多只是转发,不使用。改成 Context 之后,新增一个字段不再需要
// 修改沿途每一个签名。
//
// value 必须是 memo 化的:详情区与资源栏都有 memo 组件,靠"这一域没变就不
// 重渲染"躲开无关交互引起的重绘。每次渲染新建 value 会把这层优化全部作废。
import { createContext, useContext } from "react";

const BrowserStateContext = createContext(null);

export function BrowserStateProvider({ value, children }) {
  return (
    <BrowserStateContext.Provider value={value}>
      {children}
    </BrowserStateContext.Provider>
  );
}

export function useBrowserState() {
  const value = useContext(BrowserStateContext);
  if (!value) {
    throw new Error("useBrowserState 必须在 BrowserStateProvider 内使用");
  }
  return value;
}
