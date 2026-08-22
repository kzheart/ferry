// 标题栏工具条:侧栏开关 + 当前工作区的动作按钮 + 可拖拽留白。
import { useTranslation } from "react-i18next";
import { SidebarIcon } from "../shared/ui/icons.jsx";

// 侧栏只有「在」与「不在」两态(导航栏 + 资源栏一起收),所以只需要一颗开关。
// ⌘⇧S 之外的那条明路,不然收起来了就没人知道怎么展开。
export function WorkspaceToolbar({ collapsed, onToggle }) {
  const { t } = useTranslation();
  return (
    <>
      <button type="button" className="hov ftool-btn" data-guide="sidebar-toggle"
        aria-pressed={!collapsed}
        onClick={onToggle}
        title={collapsed ? t("app:titlebar.expand") : t("app:titlebar.collapse")}
        style={{ width: 28, height: 26 }}>
        <SidebarIcon />
      </button>
      <div data-tauri-drag-region style={{ flex: 1, alignSelf: "stretch" }} />
    </>
  );
}
