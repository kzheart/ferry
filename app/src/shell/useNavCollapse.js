import { useEffect, useState } from "react";

const COLLAPSED_KEY = "ferry-nav-collapsed";
// 240(导航栏)+ 300(资源栏)+ 分隔条 + 主区最低 580,再留出一点余量。
// 比这窄的窗口首次打开时先把导航栏收起来,让主区一开始就够读。
const NARROW_WIDTH = 1200;

function readCollapsed() {
  try {
    const stored = localStorage.getItem(COLLAPSED_KEY);
    if (stored === "1") return true;
    if (stored === "0") return false;
  } catch {
    // 隐私模式:退回按窗口宽度判断
  }
  return typeof window !== "undefined" && window.innerWidth < NARROW_WIDTH;
}

/** 导航栏折叠态。持久化在 localStorage,首次(无记录)按窗口宽度决定。 */
export function useNavCollapse() {
  const [collapsed, setCollapsed] = useState(readCollapsed);

  useEffect(() => {
    try {
      localStorage.setItem(COLLAPSED_KEY, collapsed ? "1" : "0");
    } catch {
      // 存不进去就只在本次会话里生效
    }
  }, [collapsed]);

  return { collapsed, toggle: () => setCollapsed(value => !value) };
}

export { NARROW_WIDTH };
