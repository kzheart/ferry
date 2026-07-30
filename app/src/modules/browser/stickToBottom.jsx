// 会话尾部跟随:贴底时内容更新继续贴底;离底时提供"跳到最新"悬浮按钮。
import {
  useCallback, useEffect, useLayoutEffect, useRef, useState,
} from "react";

// 距底不超过这个像素数就算"贴底";要小于分页哨兵的 600px 提前量,
// 否则加载中间页也会被误判为该吸附。
const PIN_GAP = 48;

export function useStickToBottom(scrollRef, data, sessionKey, hasMore = false) {
  const [atBottom, setAtBottom] = useState(false);
  const pinned = useRef(false);
  // "跳到最新"按下后的追底意图:分页还没拉全时 pinned 不允许成立,
  // 靠这个标记把视口一路带到整段加载完的真正结尾。
  const chasing = useRef(false);
  const lastKey = useRef(null);
  const lastTop = useRef(0);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return undefined;
    const measure = () => {
      // 用户往回(向上)滚就取消追底,不跟人抢滚动条
      if (el.scrollTop < lastTop.current - 1) chasing.current = false;
      lastTop.current = el.scrollTop;
      const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
      // 还有未加载分页时,窗口底不是会话底:绝不吸附。否则手动下滑
      // 时新页一到就被拽向新底部,内容猛地上弹。
      pinned.current = !hasMore && gap <= PIN_GAP;
      setAtBottom(pinned.current || chasing.current);
    };
    measure();
    el.addEventListener("scroll", measure, { passive: true });
    // 内容高度的异步变化(图片加载、展开折叠)也要维持贴底
    const content = el.lastElementChild;
    const observer = typeof ResizeObserver === "undefined"
      ? null
      : new ResizeObserver(() => {
        if (pinned.current || chasing.current) el.scrollTop = el.scrollHeight;
        else measure();
      });
    if (observer && content) observer.observe(content);
    return () => {
      el.removeEventListener("scroll", measure);
      observer?.disconnect();
    };
  }, [scrollRef, sessionKey, Boolean(data), hasMore]);

  // 同一会话的数据更新:原本贴底(或在追底)就跳回新底部;换会话只重测位置
  useLayoutEffect(() => {
    const el = scrollRef.current;
    const key = data ? sessionKey : null;
    const sameSession = key !== null && lastKey.current === key;
    lastKey.current = key;
    if (!sameSession) chasing.current = false;
    if (!el) return;
    if (sameSession && (pinned.current || chasing.current)) {
      el.scrollTop = el.scrollHeight;
      lastTop.current = el.scrollTop;
      if (!hasMore) {
        chasing.current = false;
        pinned.current = true;
        setAtBottom(true);
      }
    } else {
      const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
      pinned.current = !hasMore && gap <= PIN_GAP;
      setAtBottom(pinned.current || chasing.current);
    }
  }, [scrollRef, data, sessionKey, hasMore]);

  // 瞬时跳底:平滑滚动的目标高度在点击瞬间就定死了,途中分页加载把
  // 内容撑高后会落在半路;瞬跳后由 chasing/pinned 接力贴到真正结尾。
  const scrollToBottom = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    chasing.current = hasMore;
    pinned.current = !hasMore;
    setAtBottom(true);
    el.scrollTop = el.scrollHeight;
    lastTop.current = el.scrollTop;
  }, [scrollRef, hasMore]);

  return { atBottom, scrollToBottom };
}

export function JumpToLatest({ visible, raised, onClick, title }) {
  if (!visible) return null;
  return (
    <button
      className="hov-rail"
      title={title}
      onClick={onClick}
      style={{
        position: "absolute",
        left: "50%",
        bottom: raised ? 96 : 22,
        transform: "translateX(-50%)",
        width: 34,
        height: 34,
        borderRadius: "50%",
        border: "1px solid var(--line)",
        background: "var(--pane)",
        boxShadow: "0 4px 14px rgba(0,0,0,.18)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        color: "var(--tx2)",
        cursor: "default",
        zIndex: 4,
        transition: "bottom .16s ease",
      }}
    >
      <svg viewBox="0 0 16 16" width={15} height={15}>
        <path
          d="M8 3.2v9m0 0 3.6-3.6M8 12.2 4.4 8.6"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </button>
  );
}
