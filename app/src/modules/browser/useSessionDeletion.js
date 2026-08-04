import { useCallback, useState } from "react";

import { operations } from "../operations/public.js";
import { sessionIdentity } from "./sessionAttachment.js";
import {
  applyPreparedDeletion,
  cancelPreparedDeletions,
  deleteIsBlocked,
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
      if (!result.succeeded?.length) {
        setToast({
          kind: "fail",
          title: t("app:toast.deleteFail"),
          desc: t("app:toast.deleteSkippedDesc"),
        });
        return;
      }
      setToast({
        kind: "ok",
        title: t("app:toast.deleteDone"),
        desc: t("app:toast.deleteDoneDescFinal", {
          title: session.title || session.id,
        }),
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
      if (deleteIsBlocked(prepared)) {
        void cancelPreparedDeletions([prepared], operations);
        setToast({
          kind: "fail",
          title: t("app:toast.deleteFail"),
          desc: t("app:toast.deleteProtectedDesc"),
        });
        return;
      }
      // 删除不可恢复,永远弹确认,不再有"可撤销就直删"的快捷路径
      setSessionConfirmation(prepared);
      setToast(null);
    } catch (error) {
      setToast({
        kind: "fail",
        title: t("app:toast.deleteFail"),
        desc: error.message,
      });
    }
  }, [setToast, t]);

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
        const result = (
          await applyPreparedDeletion(target, operations)
        ).result;
        discardCachedDetail(target.session);
        if (result.succeeded?.length) done += 1;
        else fail += 1;
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
