import { useEffect, useState } from "react";

const COLLAPSED_KEY = "ferry-sidebar-collapsed";

/**
 * 侧栏收起态:导航栏与资源栏一起收,不留无文字的图标轨——要么整块在,要么整块不在。
 *
 * 因此默认一律展开,不再按窗口宽度自作主张:收起之后没有任何常驻入口,
 * 窄窗口首次打开就把导航藏了,等于让人对着空主区猜。窄了用户自己收。
 */
export function useSidebarCollapse() {
  const [collapsed, setCollapsed] = useState(() => {
    try {
      return localStorage.getItem(COLLAPSED_KEY) === "1";
    } catch {
      // 隐私模式读不到就当展开
      return false;
    }
  });

  useEffect(() => {
    try {
      localStorage.setItem(COLLAPSED_KEY, collapsed ? "1" : "0");
    } catch {
      // 存不进去就只在本次会话里生效
    }
  }, [collapsed]);

  return { collapsed, toggle: () => setCollapsed(value => !value) };
}
