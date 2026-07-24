import { useCallback, useState } from "react";

import { operations } from "../operations/public.js";
import { sessionIdentity } from "./sessionAttachment.js";
import {
  applyPreparedDeletion,
  cancelPreparedDeletions,
  deleteNeedsConfirmation,
  prepareSessionDeletion,
  prepareSessionDeletions,
} from "./sessionDeletionModel.js";

export function useSessionDeletion({
  clearSelection,
  discardCachedDetail,
  doScan,
  selectedId,
  setMultiIds,
  setToast,
  t,
}) {
  const [sessionConfirmation, setSessionConfirmation] = useState(null);
  const [batchConfirmation, setBatchConfirmation] = useState(null);

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

  const applySessionDeletion = useCallback(async prepared => {
    const { session } = prepared;
    setToast({
      kind: "run",
      title: t("app:toast.deleting"),
      desc: t("app:toast.deletingDesc"),
    });
    try {
      const result = (
        await applyPreparedDeletion(prepared, operations)
      ).result;
      const key = sessionIdentity(session);
      discardCachedDetail(session);
      if (selectedId === key) clearSelection();
      doScan();
      const canRestore = result.undoable && result.recovery_id;
      setToast({
        kind: "ok",
        title: t("app:toast.deleteDone"),
        desc: t(canRestore
          ? "app:toast.deleteDoneDescUndoable"
          : "app:toast.deleteDoneDescFinal", {
          title: session.title || session.id,
        }),
        action: canRestore
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

  const requestSessionDeletion = useCallback(async session => {
    setToast({
      kind: "run",
      title: t("app:toast.preparingDelete"),
      desc: t("app:toast.preparingDeleteDesc"),
    });
    try {
      const prepared = await prepareSessionDeletion(session, operations);
      if (deleteNeedsConfirmation(prepared)) {
        setSessionConfirmation(prepared);
        setToast(null);
        return;
      }
      await applySessionDeletion(prepared);
    } catch (error) {
      setToast({
        kind: "fail",
        title: t("app:toast.deleteFail"),
        desc: error.message,
      });
    }
  }, [applySessionDeletion, setToast, t]);

  const cancelSessionDeletion = useCallback(() => {
    const prepared = sessionConfirmation;
    setSessionConfirmation(null);
    if (prepared) {
      void cancelPreparedDeletions([prepared], operations);
    }
  }, [sessionConfirmation]);

  const confirmSessionDeletion = useCallback(async () => {
    const prepared = sessionConfirmation;
    if (!prepared) return;
    setSessionConfirmation(null);
    await applySessionDeletion(prepared);
  }, [applySessionDeletion, sessionConfirmation]);

  const requestBatchDeletion = useCallback(async sessions => {
    if (!sessions.length) return;
    setToast({
      kind: "run",
      title: t("app:toast.preparingBatchDelete"),
      desc: t("app:toast.preparingBatchDeleteDesc", {
        total: sessions.length,
      }),
    });
    try {
      const prepared = await prepareSessionDeletions(sessions, operations);
      setBatchConfirmation(prepared);
      setToast(null);
    } catch (error) {
      setToast({
        kind: "fail",
        title: t("app:toast.deleteFail"),
        desc: error.message,
      });
    }
  }, [setToast, t]);

  const cancelBatchDeletion = useCallback(() => {
    const prepared = batchConfirmation;
    setBatchConfirmation(null);
    if (prepared) {
      void cancelPreparedDeletions(prepared, operations);
    }
  }, [batchConfirmation]);

  const confirmBatchDeletion = useCallback(async () => {
    const prepared = batchConfirmation;
    if (!prepared) return;
    setBatchConfirmation(null);
    let done = 0;
    let fail = 0;
    for (const target of prepared) {
      setToast({
        kind: "run",
        title: t("app:toast.batchDeleting"),
        desc: t("app:toast.batchProgress", {
          done: done + fail,
          total: prepared.length,
        }),
      });
      try {
        await applyPreparedDeletion(target, operations);
        discardCachedDetail(target.session);
        done += 1;
      } catch {
        fail += 1;
      }
    }
    if (prepared.some(({ session }) =>
      sessionIdentity(session) === selectedId)) {
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
    batchConfirmation,
    clearSelection,
    discardCachedDetail,
    doScan,
    selectedId,
    setMultiIds,
    setToast,
    t,
  ]);

  return {
    batchConfirmation,
    cancelBatchDeletion,
    cancelSessionDeletion,
    confirmBatchDeletion,
    confirmSessionDeletion,
    requestBatchDeletion,
    requestSessionDeletion,
    sessionConfirmation,
  };
}
