import { filterByFeatures } from "../../shared/capabilities/features.jsx";

// 引导步骤定义:target 对应界面上的 data-guide 锚点,view 是该步骤所属模块。
// Guide 负责测量与渲染,useOnboarding 按 view 在步骤间切换模块。
//
// 标了 feature 的步骤跟着开关走:关闭时锚点不存在,步骤一起消失;
// 消费方一律经 guideSteps(isFeatureEnabled) 取表,不直接读全量表。
export function guideSteps(isFeatureEnabled = () => false) {
  return filterByFeatures(GUIDE_STEPS, isFeatureEnabled);
}

const GUIDE_STEPS = [
  {
    target: "rail",
    view: "library",
    side: "right",
    titleKey: "onboarding:guide.railTitle",
    bodyKey: "onboarding:guide.railBody",
  },
  {
    target: "overview-kpis",
    view: "overview",
    side: "bottom",
    titleKey: "onboarding:guide.overviewTitle",
    bodyKey: "onboarding:guide.overviewBody",
  },
  {
    target: "search",
    view: "library",
    side: "right",
    titleKey: "onboarding:guide.searchTitle",
    bodyKey: "onboarding:guide.searchBody",
  },
  {
    target: "filter",
    view: "library",
    side: "right",
    titleKey: "onboarding:guide.filterTitle",
    bodyKey: "onboarding:guide.filterBody",
  },
  {
    target: "detail-actions",
    view: "library",
    side: "bottom",
    titleKey: "onboarding:guide.detailTitle",
    bodyKey: "onboarding:guide.detailBody",
  },
  {
    target: "scope",
    view: "library",
    side: "top",
    scroll: true,
    titleKey: "onboarding:guide.scopeTitle",
    bodyKey: "onboarding:guide.scopeBody",
  },
  {
    target: "migrate",
    view: "library",
    side: "bottom",
    titleKey: "onboarding:guide.migrateTitle",
    bodyKey: "onboarding:guide.migrateBody",
  },
  {
    target: "pane",
    view: "history",
    side: "right",
    titleKey: "onboarding:guide.historyTitle",
    bodyKey: "onboarding:guide.historyBody",
  },
  {
    target: "rail-askferry",
    view: "askferry",
    side: "right",
    feature: "builtin-agent",
    titleKey: "onboarding:guide.askferryTitle",
    bodyKey: "onboarding:guide.askferryBody",
  },
];
