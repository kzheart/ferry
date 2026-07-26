import { TOOL_NAME } from "../shared/contracts/tools.js";

// rail 扫描按钮的 tooltip:有进度时多行显示总量与分工具明细
export function scanProgressLabel(t, scanProgress) {
  if (!(scanProgress?.total > 0)) return t("app:titlebar.scanning");
  return [
    scanProgress.phase === "finalizing"
      ? t("app:titlebar.scanFinalizing")
      : t("app:titlebar.scanProgress", {
        done: scanProgress.processed, total: scanProgress.total,
      }),
    ...Object.entries(scanProgress.tools || {})
      .filter(([, tool]) => tool.total > 0)
      .map(([key, tool]) =>
        `${TOOL_NAME[key] || key} ${tool.processed}/${tool.total}`),
  ].join("\n");
}
