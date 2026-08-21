import { useTranslation } from "react-i18next";

import { TOOL_NAME } from "../../shared/contracts/tools.js";
import { DangerConfirm } from "../../shared/ui/DangerConfirm.jsx";
import { summarizePreparedDeletions } from "./sessionDeletionModel.js";

export function SessionDeleteConfirm({ prepared, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const sess = prepared.session;
  const subCount = (sess.tree_count || 1) - 1;
  const bullets = [
    subCount > 0 && [
      "var(--warn)",
      t("overlays:delete.bulletSub", { n: subCount }),
    ],
    ["var(--err)", t("overlays:delete.bulletIrreversible")],
  ].filter(Boolean);
  return (
    <DangerConfirm
      width={430}
      title={t("overlays:delete.title")}
      desc={t("overlays:delete.desc", {
        title: sess.title || sess.id,
        tool: TOOL_NAME[sess.tool],
      })}
      bullets={bullets}
      confirmLabel={t("overlays:delete.confirmIrreversible")}
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}

export function BatchDeleteConfirm({ prepared, onCancel, onConfirm }) {
  const { t } = useTranslation();
  const summary = summarizePreparedDeletions(prepared);
  const bullets = [
    [
      "var(--err)",
      t("overlays:delete.bulletBatchIrreversible", { n: summary.total }),
    ],
    ["var(--warn)", t("overlays:delete.bulletBatchPartial")],
  ];
  return (
    <DangerConfirm
      width={430}
      title={t("overlays:delete.batchTitle", { n: summary.total })}
      bullets={bullets}
      confirmLabel={t("overlays:delete.confirmBatch")}
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}
