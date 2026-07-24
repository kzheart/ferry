// Ferry Runtime 句柄的注入点。
//
// 它是应用级能力而非某个视图的局部状态:对话视图用它收发消息,设置页用同一个
// 句柄配置 provider / model / role。以 props 逐层下传时,中途有五层只是转交,
// 改一个入参要动一串与它无关的组件签名。
//
// 容器放在 shared 而不是 modules/askferry:这样设置页读取它不必反过来依赖
// 对话模块,依赖方向仍然是 modules → shared,句柄由主壳一处注入。
import { createContext, useContext } from "react";

const FerryRuntimeContext = createContext(null);

export function FerryRuntimeProvider({ value, children }) {
  return (
    <FerryRuntimeContext.Provider value={value}>
      {children}
    </FerryRuntimeContext.Provider>
  );
}

// 缺 Provider 时立刻抛错,而不是把 undefined 传下去等某次交互才炸。
// 隐式取值的代价就在这里:类型检查看不到它,所以失败必须发生在挂载那一刻。
export function useFerryRuntime() {
  const value = useContext(FerryRuntimeContext);
  if (!value) {
    throw new Error("useFerryRuntime 必须在 FerryRuntimeProvider 内使用");
  }
  return value;
}
