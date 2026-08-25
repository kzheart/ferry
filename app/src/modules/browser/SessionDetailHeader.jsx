// 会话详情的 sticky 头部:标题与元信息、操作按钮(接续/续聊/迁移)、子会话行
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOL_NAME, resumeDescriptor, supportsSessionResumeCli } from "../../shared/contracts/tools.js";
import { fmtSize } from "../../shared/ui/toolDisplay.js";
import { fmtTime, repoOf, sessionRef } from "./sessionModel.js";
import { writeClipboardText } from "../../platform/desktop/client.js";
import {
  BranchIcon,
  CheckIcon,
  CloseIcon,
  WarnIcon,
  CopyIcon,
  HandoffIcon,
  MigrateIcon,
  ResumeMenuIcon,
  Spinner,
  TerminalIcon,
  ToolIcon,
} from "../../shared/ui/icons.jsx";
import { ContextStatusChip } from "./SessionContext.jsx";

export default function SessionDetailHeader({
  meta,
  data,
  migrationOrigin,
  onResume,
  onResumeElsewhere,
  canResume,
  canMigrate,
  onOpenMigrate,
}) {
  const { t: tt } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(null);
  // 续聊指令的反馈只落在按钮上:ok 打勾,noskill 变警告并在 title 里说去哪装
  const [handoffState, setHandoffState] = useState(null);
  const [resumeError, setResumeError] = useState(null);
  const [resuming, setResuming] = useState(false);
  const subCount = data ? data.tree_count - 1 : 0;
  const repo = repoOf(meta.dir);
  const branch = meta.branch || "";

  // 两个「复制续聊」动作都在时合并成一个分裂按钮;只剩一个时保持原来的单按钮。
  // Cursor 没有按会话 id 的接续 CLI:仍一分二,只把左边接续命令禁用并提示。
  const resumeCli = supportsSessionResumeCli(meta.tool);
  const merged = canResume && !!onResumeElsewhere;
  const resumeUnavailable = tt("browser:session.resumeCliUnavailable");
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuClosing, setMenuClosing] = useState(false);
  const splitRef = useRef(null);
  const closeTimer = useRef(null);
  const restoreTimer = useRef(null);

  const closeMenu = () => {
    clearTimeout(closeTimer.current);
    setMenuOpen(false);
    // closing 态驱动「吸回合体」动画,播完再摘掉,避免下次展开从半路起跳
    setMenuClosing(true);
    restoreTimer.current = setTimeout(() => setMenuClosing(false), 280);
  };
  const toggleMenu = () => {
    if (menuOpen) return closeMenu();
    clearTimeout(restoreTimer.current);
    setMenuClosing(false);
    setMenuOpen(true);
  };
  // 选完不立刻收:让反馈动画(飞走→画勾→涟漪)播完、错误态留足读 tooltip 的时间
  const scheduleClose = (ms) => {
    clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(closeMenu, ms);
  };

  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e) => {
      if (!splitRef.current?.contains(e.target)) closeMenu();
    };
    const onKey = (e) => {
      if (e.key === "Escape") closeMenu();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);
  useEffect(() => () => {
    clearTimeout(closeTimer.current);
    clearTimeout(restoreTimer.current);
  }, []);

  // 拿不到接续命令时不能先报"已复制":用户粘出来会是空的,却以为是自己操作错了。
  // 成败都只落在按钮上——这是本模块既有的反馈方式(见 SessionRound / SessionImagePreview)。
  const copyResume = async () => {
    if (!resumeCli) return;
    try {
      const d = await resumeDescriptor(meta.tool, sessionRef(meta));
      await writeClipboardText(d.display_command);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
      if (merged) scheduleClose(1200);
    } catch (error) {
      setCopyError(error?.message || String(error));
      setTimeout(() => setCopyError(null), 4000);
      if (merged) scheduleClose(3600);
    }
  };

  const copyHandoff = async () => {
    const result = await onResumeElsewhere(meta);
    const kind = !result?.copied ? "fail"
      : result.noSkill ? "noskill" : "ok";
    setHandoffState({ kind, error: result?.error || "" });
    setTimeout(() => setHandoffState(null), kind === "ok" ? 1600 : 4000);
    if (merged) scheduleClose(kind === "ok" ? 1200 : 3600);
  };

  const resumeInTerminal = async () => {
    if (!resumeCli || resuming) return;
    setResuming(true);
    setResumeError(null);
    try {
      await onResume(meta);
    } catch (error) {
      setResumeError(error?.message || String(error));
      setTimeout(() => setResumeError(null), 4000);
    } finally {
      setResuming(false);
    }
  };

  const copyFb = copied ? "ok" : copyError ? "err" : null;
  const handoffFb = handoffState?.kind === "ok" ? "ok"
    : handoffState?.kind === "noskill" ? "warn"
      : handoffState?.kind === "fail" ? "err" : null;
  const copyTitle = !resumeCli
    ? resumeUnavailable
    : copyError
      ? tt("browser:session.copyResumeFailed", { error: copyError })
      : copied
        ? tt("browser:session.copiedResume")
        : tt("browser:session.copyResume");
  const terminalTitle = !resumeCli
    ? resumeUnavailable
    : resumeError
      ? tt("browser:session.resumeFailed", { error: resumeError })
      : resuming
        ? tt("browser:session.resumingTerminal")
        : tt("browser:session.resumeTerminal");
  const handoffTitle = handoffState?.kind === "fail"
    ? tt("browser:session.copyResumeElsewhereFailed", {
      error: handoffState.error,
    })
    : handoffState?.kind === "noskill"
      ? tt("browser:session.copiedResumeElsewhereNoSkill")
      : handoffState?.kind === "ok"
        ? tt("browser:session.copiedResumeElsewhere")
        : tt("browser:session.copyResumeElsewhere");

  return (
    <div
      style={{
        padding: "18px var(--main-pad) 14px",
        borderBottom: "1px solid var(--line5)",
        position: "sticky",
        top: 0,
        background: "var(--veil)",
        backdropFilter: "blur(6px)",
        zIndex: 2,
      }}
    >
      <div style={{ display: "flex", alignItems: "flex-start", gap: 13 }}>
        <ToolIcon tool={meta.tool} size={40} dot="var(--ok)" />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              fontSize: "var(--fs-detail-title)",
              fontWeight: 600,
              letterSpacing: "-.01em",
            }}
          >
            {meta.title || tt("browser:session.untitled")}
          </div>
          <div
            style={{
              display: "flex",
              flexWrap: "wrap",
              alignItems: "center",
              gap: "6px 14px",
              marginTop: 6,
              fontSize: "var(--fs-meta)",
              color: "var(--tx3b)",
            }}
          >
            <span>
              {tt("browser:session.source")}{" "}
              <b style={{ color: "var(--tx2)", fontWeight: 600 }}>
                {TOOL_NAME[meta.tool]}
              </b>
            </span>
            {/* 迁移产物的出处:历史页已移除,来源就落在这条元信息里 */}
            {migrationOrigin && (
              <span
                title={fmtTime(migrationOrigin.time, tt)}
                style={{ display: "inline-flex", alignItems: "center", gap: 4,
                  color: "var(--tx4)" }}
              >
                <MigrateIcon size={12} />
                {tt("browser:session.migratedFrom", {
                  tool: TOOL_NAME[migrationOrigin.src] || migrationOrigin.src,
                })}
              </span>
            )}
            {(repo || branch) && (
              <span
                title={meta.dir || undefined}
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 5,
                  color: "var(--tx4)",
                  minWidth: 0,
                }}
              >
                {repo && <span className="mono">{repo}</span>}
                {branch && (
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 3 }}>
                    <BranchIcon size={12} />
                    <span className="mono">{branch}</span>
                  </span>
                )}
              </span>
            )}
            <span>
              {tt("browser:session.messages", {
                n: data ? data.count : meta.count,
              })}
            </span>
            <ContextStatusChip context={data?.context} />
            <span>{fmtSize(meta.size)}</span>
            <span>
              {tt("browser:session.active", {
                time: fmtTime(meta.updated, tt),
              })}
            </span>
          </div>
        </div>
        <div
          data-guide="detail-actions"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 2,
            flex: "none",
          }}
        >
          {canResume && <button
            className="ftool-btn"
            onClick={resumeInTerminal}
            disabled={!resumeCli || resuming}
            title={terminalTitle}
            style={resumeError ? { color: "var(--err)" } : undefined}
          >
            {resuming ? <Spinner size={14} />
              : resumeError ? <WarnIcon size={15} /> : <TerminalIcon />}
          </button>}
          {/* 续聊:两个「复制」动作(接续命令/续聊指令)都是把这条会话接着聊下去,
              只差换不换 agent,合并成一个按钮,点击一分为二向下摊开 */}
          {merged ? (
            <div
              ref={splitRef}
              className={"fsplit" + (menuOpen ? " open" : menuClosing ? " closing" : "")}
            >
              <button
                data-guide="handoff"
                className="ftool-btn fsplit-trigger"
                title={tt("browser:session.resumeMenu")}
                aria-expanded={menuOpen}
                aria-haspopup="true"
                onClick={toggleMenu}
              >
                <span className="fsplit-glyph"><ResumeMenuIcon /></span>
                <span className="fsplit-cross"><CloseIcon size={12} /></span>
              </button>
              <button
                className={"fsplit-opt l" + (copyFb ? ` picked picked-${copyFb}` : "")}
                title={copyTitle}
                disabled={!resumeCli}
                tabIndex={menuOpen && resumeCli ? 0 : -1}
                onClick={copyResume}
              >
                <span className="oi"><CopyIcon size={14} /></span>
                {copyFb && <span className="fb">
                  {copyFb === "ok" ? <CheckIcon size={14} /> : <WarnIcon size={14} />}
                </span>}
              </button>
              <button
                className={"fsplit-opt r" + (handoffFb ? ` picked picked-${handoffFb}` : "")}
                title={handoffTitle}
                tabIndex={menuOpen ? 0 : -1}
                onClick={copyHandoff}
              >
                <span className="oi"><HandoffIcon /></span>
                {handoffFb && <span className="fb">
                  {handoffFb === "ok" ? <CheckIcon size={14} /> : <WarnIcon size={14} />}
                </span>}
              </button>
            </div>
          ) : (
            <>
              {canResume && <button
                className="ftool-btn"
                onClick={copyResume}
                disabled={!resumeCli}
                title={copyTitle}
                style={copied ? { color: "var(--ok)" }
                  : copyError ? { color: "var(--err)" } : undefined}
              >
                {copied ? <CheckIcon size={15} />
                  : copyError ? <WarnIcon size={15} /> : <CopyIcon size={15} />}
              </button>}
              {onResumeElsewhere && <button
                data-guide="handoff"
                className="ftool-btn"
                title={handoffTitle}
                onClick={copyHandoff}
                style={
                  handoffState?.kind === "ok" ? { color: "var(--ok)" }
                    : handoffState?.kind === "noskill" ? { color: "var(--warn)" }
                      : handoffState?.kind === "fail" ? { color: "var(--err)" }
                        : undefined
                }
              >
                {handoffState?.kind === "ok" ? <CheckIcon size={15} />
                  : handoffState ? <WarnIcon size={15} /> : <HandoffIcon />}
              </button>}
            </>
          )}
          {canMigrate && (
            <button
              data-guide="migrate"
              className="ftool-btn"
              title={tt("browser:session.migrate")}
              onClick={() => onOpenMigrate(null)}
            >
              <MigrateIcon />
            </button>
          )}
        </div>
      </div>
      {subCount > 0 && (
        <div
          className="mono"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 7,
            marginTop: 13,
            fontSize: 11,
            color: "var(--tx4)",
          }}
        >
          <span
            style={{
              padding: "2px 8px",
              borderRadius: 6,
              background: "var(--chip)",
              color: "var(--tx3b)",
            }}
          >
            {tt("browser:session.subSessionsLine", {
              tool: TOOL_NAME[meta.tool],
            })}
          </span>
          <span>{tt("browser:session.arrow")}</span>
          <span
            style={{
              padding: "2px 8px",
              borderRadius: 6,
              color: "var(--tx3b)",
            }}
          >
            {tt("browser:session.subSessions", { n: subCount })}
          </span>
        </div>
      )}
    </div>
  );
}
