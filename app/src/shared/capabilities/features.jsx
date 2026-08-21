// 特性开关在前端的那一份显示副本。
//
// 事实源是宿主的配置文件(宿主的门自己回读它,不信 WebView),这里只解决一件事:
// 导航轨、工作区路由、设置页、右键菜单、引导步骤同时看这些值,切换要立刻同步,
// 所以副本必须是模块级的一份,而不是各自 useState 一份——也因此 hook 在树里的
// 任何位置都能用,不必层层包 Context。
//
// 契约(contracts/features.json 的生成物)只提供「有哪些特性、默认是什么」;
// 「这台机器上开没开」永远来自宿主。
import { useCallback, useEffect, useState } from "react";

import { FEATURES } from "../contracts/generated/features.js";
import { featureSet, featuresList } from "../../platform/desktop/client.js";

// 回读落地之前用契约默认:宁可少显示一个入口,也不要闪一下再消失。
const CONTRACT_DEFAULTS = FEATURES.map(feature => ({
  ...feature,
  enabled: feature.default,
}));

let current = CONTRACT_DEFAULTS;
let inflight = null;
const listeners = new Set();

function publish(next) {
  current = next;
  for (const listener of [...listeners]) listener(current);
}

/**
 * 回读宿主的快照。同一拍里多个消费方挂载时共用一次调用,挂载之后再挂的会重新读
 * ——开关也可能被这个进程之外的东西改过(直接编辑配置文件)。
 */
export function refreshFeatures() {
  if (!inflight) {
    inflight = featuresList()
      .then(states => {
        if (Array.isArray(states)) publish(states);
      })
      .catch(() => {})
      .finally(() => { inflight = null; });
  }
  return inflight;
}

/** 改一个特性:先落宿主,成功了才更新本地副本,全部消费点随之同步。 */
export async function setFeature(id, enabled) {
  await featureSet(id, enabled);
  publish(current.map(feature =>
    (feature.id === id ? { ...feature, enabled } : feature)));
}

function useFeatureStates() {
  const [states, setStates] = useState(current);
  useEffect(() => {
    listeners.add(setStates);
    setStates(current);
    refreshFeatures();
    return () => {
      listeners.delete(setStates);
    };
  }, []);
  return states;
}

/** 设置页用:契约里全部有界面那一面的特性,连同当前值。 */
export function useFeaturesList() {
  return useFeatureStates();
}

/** 单个特性的开关值。 */
export function useFeature(id) {
  const states = useFeatureStates();
  return Boolean(states.find(feature => feature.id === id)?.enabled);
}

/** 列表型入口用:一个可直接喂给 {@link filterByFeatures} 的判定函数。 */
export function useIsFeatureEnabled() {
  const states = useFeatureStates();
  return useCallback(
    id => Boolean(states.find(feature => feature.id === id)?.enabled),
    [states],
  );
}

/**
 * 列表型入口的声明式过滤:表项可选地标一个 `feature`,没标的恒显示。
 *
 * 导航轨、设置分区、引导步骤、右键菜单四处都走这一个 helper,所以「某个入口
 * 忘了跟着开关消失」只可能是表里漏标,而不是某处过滤逻辑写错。
 */
export function filterByFeatures(items, isFeatureEnabled) {
  return items.filter(item => !item?.feature || isFeatureEnabled(item.feature));
}

/**
 * 只做一件事:在主壳挂载之前先发起一次回读,让首帧尽早拿到宿主的快照。订阅本身
 * 是模块级的,所以它不注入任何 Context,子树里的 hook 不依赖它存在。
 */
export function FeaturesProvider({ children }) {
  useEffect(() => { refreshFeatures(); }, []);
  return children;
}
