import { useCallback, useEffect, useState } from "react";

import { engine } from "../../platform/desktop/client.js";
import { operations } from "../operations/operations.js";
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
    } catch (error) {
      setToast({
        kind: "fail",
        title: t("app:toast.metaSaveFail"),
        desc: error.message,
      });
    }
  }, [setToast, t]);

  return {
    metadata,
    metaFor,
    reloadMetadata,
    updateMetadata,
  };
}
