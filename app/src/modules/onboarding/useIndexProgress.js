// 轮询引擎 scan_progress:一次拿到两件事——元数据扫描进度(会话列表何时可用)
// 和内容索引覆盖度(全文搜索何时完整)。首启向导用快轮询,标题栏胶囊用慢轮询。
// 不算预计剩余时间:索引速度随会话大小剧烈波动,ETA 来回跳反而伤信任。
import { useEffect, useState } from "react";
import { engine } from "../../platform/desktop/client.js";

export function useIndexProgress({ active = true, interval = 2000 } = {}) {
  const [progress, setProgress] = useState(null);

  useEffect(() => {
    if (!active) return undefined;
    let stopped = false;
    const tick = async () => {
      try {
        const payload = await engine("scan_progress");
        if (!stopped) setProgress(payload);
      } catch {
        // 引擎不可达(浏览器开发预览):没有进度就是没有,调用方自己降级
        if (!stopped) setProgress(null);
      }
    };
    tick();
    const timer = setInterval(tick, interval);
    return () => { stopped = true; clearInterval(timer); };
  }, [active, interval]);

  const contentIndex = progress?.content_index && typeof progress.content_index === "object"
    ? progress.content_index
    : null;

  return { progress, contentIndex };
}
