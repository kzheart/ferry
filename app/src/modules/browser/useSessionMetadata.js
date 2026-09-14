import { useCallback, useEffect, useState } from "react";

import { engine } from "../../platform/desktop/client.js";
import { operations } from "../operations/public.js";
import { operationRef } from "./sessionModel.js";
import { sessionIdentity } from "./sessionAttachment.js";

export function useSessionMetadata({ setToast, t }) {
  const [metadata, setMetadata] = useState({});

  const reloadMetadata = useCallback(() =>
    engine("session_meta_list")
      .then(value => setMetadata(value || {}))
      .catch(() => {}), []);

  useEffect(() => {
    reloadMetadata();
  }, [reloadMetadata]);

  const metaFor = useCallback(
    session => metadata[sessionIdentity(session)] || {},
    [metadata],
  );

  const updateMetadata = useCallback(async (session, patch) => {
    try {
      const plan = await operations.plan({
        kind: "metadata",
        tool: session.tool,
        ref: operationRef(session),
        patch,
      });
      const applied = await operations.apply(plan);
      const entry = applied.result.metadata;
      setMetadata(current => {
        const next = { ...current };
        const key = sessionIdentity(session);
        if (entry && Object.keys(entry).length) next[key] = entry;
        else delete next[key];
        return next;
      });
      return true;
    } catch (error) {
      setToast({
        kind: "fail",
        title: t("app:toast.metaSaveFail"),
        desc: error.message,
      });
      return false;
    }
  }, [setToast, t]);

  // 原生标题写回:走 rename operation,由 engine 写进 Agent 自己的存储并清掉本地
  // name 覆盖。错误交给调用方提示(文案随 Agent 不同)。返回 result 供展示 notes。
  const renameSession = useCallback(async (session, title) => {
    const plan = await operations.plan({
      kind: "rename",
      tool: session.tool,
      ref: operationRef(session),
      title,
    });
    const applied = await operations.apply(plan);
    const entry = applied.result?.metadata;
    setMetadata(current => {
      const next = { ...current };
      const key = sessionIdentity(session);
      if (entry && Object.keys(entry).length) next[key] = entry;
      else delete next[key];
      return next;
    });
    return applied.result || {};
  }, []);

  return {
    metadata,
    metaFor,
    reloadMetadata,
    updateMetadata,
    renameSession,
  };
}
