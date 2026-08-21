// 界面密度:standard(默认,「标准/大气」)与 compact(改版前的紧凑数值)。
//
// 排版本身走 app.css 里挂在 html[data-density] 上的 CSS 变量;这里额外维护一份
// 同样的数值表,是因为资源栏是虚拟列表——它得先按数字把每行的 y 与总高算出来,
// 才知道哪些行要挂载,而 CSS 变量在 JS 里拿不到(getComputedStyle 每帧读一次太重)。
// 两份数值必须一起改:app.css 的 :root / html[data-density="compact"] 两块。
import { useEffect, useState } from "react";

export const DENSITY_KEY = "ferry-density";
export const DENSITIES = ["compact", "standard"];
export const DEFAULT_DENSITY = "standard";
// 密度变了要重排虚拟列表:设置页改完广播一次,各处 useDensityMetrics 跟着重算
export const DENSITY_EVENT = "ferry-density-change";

export const DENSITY_METRICS = {
  standard: {
    navRow: 32,      // 导航栏行高
    navIcon: 17,     // 导航栏 / 列表图标
    folderRow: 34,   // 资源栏项目文件夹头
    libRow: 48,      // 资源栏会话行(双行)
    histRow: 34,     // 迁移历史行(单行)
    groupHeader: 28, // 时间分组标题行
    paneDefault: 300,
    paneMin: 240,
    paneMax: 380,
  },
  compact: {
    navRow: 28,
    navIcon: 15,
    folderRow: 30,
    libRow: 40,
    histRow: 30,
    groupHeader: 24,
    paneDefault: 250,
    paneMin: 190,
    paneMax: 320,
  },
};

export function normalizeDensity(value) {
  return DENSITIES.includes(value) ? value : DEFAULT_DENSITY;
}

export function readDensity() {
  try {
    return normalizeDensity(localStorage.getItem(DENSITY_KEY));
  } catch {
    // 隐私模式读不到就用默认
    return DEFAULT_DENSITY;
  }
}

/** 打到根节点上,CSS 变量随之切换;同时广播给需要数值的虚拟列表。 */
export function applyDensity(value) {
  const density = normalizeDensity(value);
  if (typeof document !== "undefined") {
    document.documentElement.dataset.density = density;
  }
  return density;
}

export function writeDensity(value) {
  const density = applyDensity(value);
  try {
    localStorage.setItem(DENSITY_KEY, density);
  } catch {
    // 存不进去就只在本次会话里生效
  }
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent(DENSITY_EVENT, { detail: density }));
  }
  return density;
}

/** 当前密度(跟随切换),给设置页的分段控件与需要重算的组件用。 */
export function useDensity() {
  const [density, setDensity] = useState(readDensity);
  useEffect(() => {
    const sync = () => setDensity(readDensity());
    window.addEventListener(DENSITY_EVENT, sync);
    return () => window.removeEventListener(DENSITY_EVENT, sync);
  }, []);
  return density;
}

/** 当前密度下的一组行高/栏宽数值。 */
export function useDensityMetrics() {
  return DENSITY_METRICS[useDensity()];
}
