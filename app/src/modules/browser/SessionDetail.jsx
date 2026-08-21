// 会话详情:头部 + 会话树 chips + 按轮时间线;轮次操作 hover 显现,有暂存操作时底部浮出操作条
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  supportsEditOperation,
  supportsAgentCapability,
  TOOLS,
} from "../../shared/contracts/tools.js";
import { useSessionEditingSurface } from "../../shared/capabilities/sessionEditing.jsx";
import { fmtSize } from "../../shared/ui/toolDisplay.js";
import { toRounds, toTimeline } from "./sessionModel.js";
import { Spinner } from "../../shared/ui/icons.jsx";
import PendingEditBar from "./PendingEditBar.jsx";
import { CompactionBoundary } from "./SessionContext.jsx";
import SessionDetailHeader from "./SessionDetailHeader.jsx";
import SessionImagePreview from "./SessionImagePreview.jsx";
import SessionRound from "./SessionRound.jsx";
import {
  OptimizationAgentBar,
  OptimizationFloatBar,
  OptimizationMinimap,
  OptimizationNotice,
} from "./OptimizationSurface.jsx";
import { useOptimizationView } from "./useOptimizationView.js";
import { JumpToLatest, useStickToBottom } from "./stickToBottom.jsx";

// memo:侧边栏展开/折叠、悬停等与详情无关的状态变化不再重渲染整条时间线
export default memo(function SessionDetail({
  meta,
  data,
  error,
  onOpenMigrate,
  onRefresh,
  refreshing,
  onResume,
  navigationTarget,
  onLoadMore,
  loadingMore,
  optimization,
}) {
  const { t: tt } = useTranslation();
  const {
    scope, setScope, ops, dirtyOps, addOp, removeOp, updateOp,
    startReplyEdit, replyEditError, onOpenDiff, onApply, applying, onDiscardAll,
  } = useSessionEditingSurface();
  const rounds = useMemo(() => toRounds(data?.messages, data?.turns), [data]);
  const timeline = useMemo(
    () => toTimeline(rounds, data?.context_compactions, Boolean(data?.next_from_message)),
    [rounds, data?.context_compactions, data?.next_from_message],
  );
  const canDelete = supportsAgentCapability(meta.tool, "edit")
    && supportsEditOperation(meta.tool, "delete-turn");
  const canRewrite = supportsEditOperation(meta.tool, "rewrite");
  const canEditReply = supportsEditOperation(
    meta.tool,
    "replace-assistant-reply",
  );
  const canMigrate = TOOLS.includes(meta.tool)
    && supportsAgentCapability(meta.tool, "migration-source");
  const canResume = supportsAgentCapability(meta.tool, "resume");
  const loadMoreRef = useRef(null);

  // 无感分页:哨兵接近视口(提前 600px)即触发加载,加载完成后 next_from_message
  // 变化会重建 observer,若哨兵仍在视口内则继续加载直到填满
  useEffect(() => {
    const el = loadMoreRef.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) onLoadMore();
      },
      { rootMargin: "600px 0px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [data?.next_from_message, onLoadMore]);
  const [previewImages, setPreviewImages] = useState(null);
  const scrollRef = useRef(null);
  const { atBottom, scrollToBottom } = useStickToBottom(
    scrollRef, data, meta.id, Boolean(data?.next_from_message),
  );

  // 会话优化的视图状态(候选映射/多选/乐观展示/跳转/快捷键)收在专用 hook
  const optView = useOptimizationView({
    optimization,
    rounds,
    data,
    metaId: meta.id,
    canRewrite,
    scrollRef,
  });
  const {
    optActive, candidates, pendingTurns, acceptAllCandidates,
    selectedTurns, setSelectedTurns, startOptimization, jumpToTurn, navDiff,
  } = optView;

  // 每个导航目标只定位一次:运行中会话每次内容重载都会换 data,
  // 不记账会反复滚回目标轮次,把停在底部的用户周期性甩上去。
  const handledNavRef = useRef(null);
  useEffect(() => {
    if (!data || navigationTarget?.view !== "library") return;
    if (handledNavRef.current === navigationTarget) return;
    const round = Number(navigationTarget.turn);
    if (!Number.isFinite(round) || round < 1) return;
    const el = document.querySelector(`[data-round="${round}"]`);
    if (!el) return; // 目标轮次还没分页加载进来,等下次 data 变化重试
    handledNavRef.current = navigationTarget;
    requestAnimationFrame(() =>
      el.scrollIntoView({ behavior: "smooth", block: "center" }),
    );
  }, [data, navigationTarget]);

  const roundSize = (r) =>
    (r.user?.length || 0) +
    r.ai.join("").length +
    r.tools.reduce((a, t) => a + (t.size || 0), 0);

  const scopeMsgs = scope
    ? rounds
        .slice(0, scope)
        .reduce((a, r) => a + 1 + (r.ai.length ? 1 : 0), 0) +
      rounds.slice(0, scope).reduce((a, r) => a + r.tools.length, 0)
    : 0;
  const scopeStats = scope
    ? tt("browser:round.scopeStats", {
        msgs: scopeMsgs,
        size: fmtSize(
          rounds.slice(0, scope).reduce((a, r) => a + roundSize(r), 0),
        ),
      })
    : "";

  const opFor = (n, type) => ops.find((o) => o.type === type && o.n === n);

  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        minWidth: 0,
        minHeight: 0,
        position: "relative",
      }}
    >
      <div
        ref={scrollRef}
        className="fscroll"
        data-guide-scroll="1"
        style={{ flex: 1, overflowY: "auto", minWidth: 0 }}
      >
        <SessionDetailHeader
          meta={meta}
          data={data}
          refreshing={refreshing}
          onRefresh={onRefresh}
          onResume={onResume}
          canResume={canResume}
          canMigrate={canMigrate}
          onOpenMigrate={onOpenMigrate}
          optActive={optActive}
          optimization={optimization}
          onStartOptimization={() => startOptimization()}
        />
        {optActive && (
          <>
            <OptimizationAgentBar optimization={optimization} />
            <OptimizationNotice optimization={optimization} />
          </>
        )}
        <div
          style={{
            padding: `20px var(--main-pad) ${dirtyOps.length ? 110 : 48}px`,
            maxWidth: "var(--read-max)",
            margin: "0 auto",
          }}
        >
          {error && (
            <div
              style={{ padding: 30, color: "var(--err-deep)", fontSize: 13 }}
            >
              {tt("browser:session.readFailed", { error })}
            </div>
          )}
          {!data && !error && (
            <div
              style={{
                padding: 40,
                display: "flex",
                alignItems: "center",
                gap: 10,
                color: "var(--tx4)",
                fontSize: 13,
              }}
            >
              <Spinner size={16} /> {tt("browser:session.parsing")}
            </div>
          )}
          {data &&
            timeline.map((item) => {
              if (item.kind === "compaction") {
                return (
                  <CompactionBoundary
                    key={item.key}
                    compactions={item.compactions}
                  />
                );
              }
              const r = item.round;
              return (
                <SessionRound
                  key={item.key}
                  r={r}
                  canDelete={canDelete}
                  canRewrite={canRewrite}
                  delOp={opFor(r.n, "delete")}
                  rewOp={opFor(r.n, "rewrite")}
                  replyOp={opFor(r.n, "assistant-reply")}
                  canEditReply={canEditReply && !!r.assistantReply}
                  replyEditBlocked={
                    ops.length > 0 && !opFor(r.n, "assistant-reply")
                  }
                  onDelete={() => addOp("delete", r)}
                  onUndoDelete={() => {
                    const o = opFor(r.n, "delete");
                    if (o) removeOp(o.id);
                  }}
                  onRewrite={() => addOp("rewrite", r)}
                  onUpdateRewrite={(text) => {
                    const o = opFor(r.n, "rewrite");
                    if (o) updateOp(o.id, { text });
                  }}
                  onCancelRewrite={() => {
                    const o = opFor(r.n, "rewrite");
                    if (o) removeOp(o.id);
                  }}
                  {...optView.roundProps(r)}
                  onStartReply={() => startReplyEdit(r.assistantReply)}
                  onUpdateReply={(items) => {
                    const o = opFor(r.n, "assistant-reply");
                    if (o) updateOp(o.id, { items });
                  }}
                  onCancelReply={() => {
                    const o = opFor(r.n, "assistant-reply");
                    if (o) removeOp(o.id);
                  }}
                  migratable={canMigrate && r.n < rounds.length}
                  scopeOn={scope === r.n}
                  onScope={() => setScope(r.n)}
                  onClearScope={() => setScope(null)}
                  onMigrateScope={() => onOpenMigrate(r.n)}
                  scopeStats={scopeStats}
                  onOpenImages={setPreviewImages}
                />
              );
            })}
          {data && rounds.length === 0 && (
            <div style={{ padding: 30, color: "var(--tx5)", fontSize: 12 }}>
              {tt("browser:session.noMessages")}
            </div>
          )}
          {data?.next_from_message && (
            <div
              ref={loadMoreRef}
              style={{
                display: "flex",
                justifyContent: "center",
                alignItems: "center",
                height: 40,
                color: "var(--tx4)",
              }}
            >
              {loadingMore && <Spinner size={14} />}
            </div>
          )}
        </div>
      </div>
      <JumpToLatest
        visible={Boolean(data) && !atBottom}
        raised={dirtyOps.length > 0}
        onClick={() => {
          // 先把剩余分页一次拉全再贴底,否则"底部"只是当前窗口的底
          if (data?.next_from_message) onLoadMore(true);
          scrollToBottom();
        }}
        title={tt("browser:session.jumpToLatest")}
      />
      {optActive && (
        <>
          <OptimizationFloatBar
            pendingCount={candidates.length}
            applying={optimization.status === "applying"}
            onPrev={() => navDiff(-1)}
            onNext={() => navDiff(1)}
            onAcceptAll={acceptAllCandidates}
            onRejectAll={optimization.rejectAll}
            selection={
              optimization.role
                ? {
                    count: selectedTurns.length,
                    roleName: optimization.role.name,
                    roleColor: optimization.role.color,
                    onCancel: () => setSelectedTurns([]),
                    onRun: () => startOptimization(selectedTurns),
                  }
                : null
            }
          />
          <OptimizationMinimap
            scrollRef={scrollRef}
            pendingTurns={pendingTurns}
            onJump={jumpToTurn}
          />
        </>
      )}
      {dirtyOps.length > 0 && (
        <PendingEditBar
          ops={dirtyOps}
          removeOp={removeOp}
          onOpenDiff={onOpenDiff}
          onApply={onApply}
          applying={applying}
          invalid={replyEditError(
            dirtyOps.find((op) => op.type === "assistant-reply"),
          )}
          onDiscardAll={onDiscardAll}
        />
      )}
      {previewImages && (
        <SessionImagePreview
          key={previewImages[0]?.id}
          images={previewImages}
          meta={meta}
          onClose={() => setPreviewImages(null)}
        />
      )}
    </div>
  );
});
