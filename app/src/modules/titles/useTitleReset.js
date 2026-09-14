// 「AI 重置标题」的流程:取证 → 生成 → 预览编辑 → 逐条写回 → 汇总 + 重扫。
// 写回复用既有 rename operation(Cursor 没有原生改名,落到本地元数据 name)。
import { useCallback, useEffect, useRef, useState } from "react";

import { supportsAgentCapability } from "../../shared/contracts/tools.js";
import { sessionRef } from "../browser/public.js";
import {
  applicableTitleRows,
  buildTitleRows,
  dedupeNotes,
  titleRowKey,
} from "./titleResetModel.js";
import { generateTitles } from "./titleStyle.js";

export function useTitleReset({
  sessions,
  batch,
  renameSession,
  updateMetadata,
  rescan,
  setToast,
  onClose,
  t,
}) {
  // 单选是用户对着这一条点的,手动命名也照改;批量默认放过手动命名的。
  const [includeManual, setIncludeManual] = useState(!batch);
  const [status, setStatus] = useState("loading");
  const [error, setError] = useState(null);
  const [model, setModel] = useState(null);
  const [rows, setRows] = useState([]);
  const request = useRef(0);
  const sessionList = useRef(sessions);
  sessionList.current = sessions;

  const generate = useCallback(async manual => {
    const ticket = ++request.current;
    setStatus("loading");
    setError(null);
    const list = sessionList.current || [];
    const sessionByKey = new Map(
      list.map(session => [titleRowKey(session.tool, sessionRef(session)), session]),
    );
    try {
      const { evidence, items, model: used } = await generateTitles({
        sessions: list,
        includeManual: manual,
        style: null,
      });
      if (ticket !== request.current) return;
      setModel(used);
      setRows(buildTitleRows({ evidence, items, includeManual: manual, sessionByKey }));
      setStatus("ready");
    } catch (failure) {
      if (ticket !== request.current) return;
      setError({
        code: failure?.code || null,
        message: String(failure?.message || failure || ""),
      });
      setStatus("error");
    }
  }, []);

  useEffect(() => {
    generate(includeManual);
  }, [generate, includeManual]);

  const updateRow = useCallback((key, patch) => {
    setRows(current => current.map(row => (row.key === key ? { ...row, ...patch } : row)));
  }, []);

  const apply = useCallback(async () => {
    const pending = applicableTitleRows(rows);
    if (!pending.length) {
      onClose?.();
      return;
    }
    setStatus("applying");
    let ok = 0;
    let failed = 0;
    const notes = [];
    for (const row of pending) {
      const title = row.after.trim();
      try {
        if (supportsAgentCapability(row.tool, "rename")) {
          const result = await renameSession(row.session, title);
          notes.push(...(result?.native?.notes || []));
        } else {
          // updateMetadata 会自行提示并返回 false，不会抛出写回错误。
          if (!await updateMetadata(row.session, { name: title })) {
            failed += 1;
            continue;
          }
        }
        ok += 1;
      } catch {
        failed += 1;
      }
    }
    setToast?.({
      kind: failed ? "fail" : "ok",
      title: failed
        ? t("app:toast.titleResetPartial", { ok, failed })
        : t("app:toast.titleReset", { n: ok }),
      desc: [t("app:toast.titleResetDesc", { n: ok }), ...dedupeNotes(notes)].join("，"),
    });
    rescan?.();
    onClose?.();
  }, [rows, renameSession, updateMetadata, rescan, setToast, onClose, t]);

  return {
    status,
    error,
    model,
    rows,
    includeManual,
    setIncludeManual,
    updateRow,
    apply,
    selectedCount: applicableTitleRows(rows).length,
    retry: () => generate(includeManual),
  };
}
