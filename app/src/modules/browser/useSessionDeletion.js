import { useCallback } from "react";

import { operations } from "../operations/operations.js";
import { operationRef } from "./sessionModel.js";
import { sessionIdentity } from "./sessionAttachment.js";

export function useSessionDeletion({
  clearSelection,
  discardCachedDetail,
  doScan,
  selectedId,
  setMultiIds,
  setToast,
  t,
}) {
  const restoreSession = useCallback(async recoveryId => {
    setToast({
      kind: "run",
      title: t("app:toast.restoring"),
      desc: t("app:toast.restoringDesc"),
    });
    try {
      const plan = await operations.plan({
        kind: "restore-delete",
        recovery_id: recoveryId,
      });
      await operations.apply(plan);
      doScan();
      setToast({
        kind: "ok",
        title: t("app:toast.restoreDone"),
        desc: t("app:toast.restoreDoneDesc"),
      });
    } catch (error) {
      setToast({
        kind: "fail",
        title: t("app:toast.restoreFail"),
        desc: error.message,
      });
    }
  }, [doScan, setToast, t]);

  const deleteSession = useCallback(async session => {
    setToast({
      kind: "run",
      title: t("app:toast.deleting"),
      desc: t("app:toast.deletingDesc"),
    });
    try {
      const plan = await operations.plan({
        kind: "delete",
        tool: session.tool,
        ref: operationRef(session),
      });
      const result = (await operations.apply(plan)).result;
      const key = sessionIdentity(session);
      discardCachedDetail(session);
      if (selectedId === key) clearSelection();
      doScan();
      setToast({
        kind: "ok",
        title: t("app:toast.deleteDone"),
        desc: t("app:toast.deleteDoneDesc", {
          title: session.title || session.id,
        }),
        action: result.undoable
          ? {
              label: t("app:toast.undo"),
              onClick: () => restoreSession(result.recovery_id),
            }
          : undefined,
      });
    } catch (error) {
      setToast({
        kind: "fail",
        title: t("app:toast.deleteFail"),
        desc: error.message,
      });
    }
  }, [
    clearSelection,
    discardCachedDetail,
    doScan,
    restoreSession,
    selectedId,
    setToast,
    t,
  ]);

  const deleteSessions = useCallback(async targets => {
    let done = 0;
    let fail = 0;
    for (const session of targets) {
      setToast({
        kind: "run",
        title: t("app:toast.batchDeleting"),
        desc: t("app:toast.batchProgress", {
          done: done + fail,
          total: targets.length,
        }),
      });
      try {
        const plan = await operations.plan({
          kind: "delete",
          tool: session.tool,
          ref: operationRef(session),
        });
        await operations.apply(plan);
        discardCachedDetail(session);
        done += 1;
      } catch {
        fail += 1;
      }
    }
    if (targets.some(session => sessionIdentity(session) === selectedId)) {
      clearSelection();
    }
    setMultiIds([]);
    doScan();
    setToast(fail
      ? {
          kind: "fail",
          title: t("app:toast.batchPartialFail"),
          desc: t("app:toast.batchPartialFailDesc", { done, fail }),
        }
      : {
          kind: "ok",
          title: t("app:toast.batchDone"),
          desc: t("app:toast.batchDoneDesc", { done }),
        });
  }, [
    clearSelection,
    discardCachedDetail,
    doScan,
    selectedId,
    setMultiIds,
    setToast,
    t,
  ]);

  return { deleteSession, deleteSessions };
}
