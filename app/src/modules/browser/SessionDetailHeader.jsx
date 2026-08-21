// 会话详情的 sticky 头部:标题与元信息、操作按钮(刷新/接续/复制/优化/迁移)、子会话行
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { TOOL_NAME, resumeDescriptor } from "../../shared/contracts/tools.js";
import { fmtSize } from "../../shared/ui/toolDisplay.js";
import { fmtTime, sessionRef } from "./sessionModel.js";
import { writeClipboardText } from "../../platform/desktop/client.js";
import {
  CheckIcon,
  WarnIcon,
  CopyIcon,
  MigrateIcon,
  RefreshIcon,
  Spinner,
  TerminalIcon,
  ToolIcon,
} from "../../shared/ui/icons.jsx";
import { ContextStatusChip } from "./SessionContext.jsx";
import { OptimizerWandControl } from "./OptimizationSurface.jsx";

export default function SessionDetailHeader({
  meta,
  data,
  refreshing,
  onRefresh,
  onResume,
  canResume,
  canMigrate,
  onOpenMigrate,
  optActive,
  optimization,
  onStartOptimization,
}) {
  const { t: tt } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(null);
  const [resumeError, setResumeError] = useState(null);
  const [resuming, setResuming] = useState(false);
  const subCount = data ? data.tree_count - 1 : 0;

  // 拿不到接续命令时不能先报"已复制":用户粘出来会是空的,却以为是自己操作错了。
  // 成败都只落在按钮上——这是本模块既有的反馈方式(见 SessionRound / SessionImagePreview)。
  const copyResume = async () => {
    try {
      const d = await resumeDescriptor(meta.tool, sessionRef(meta));
      await writeClipboardText(d.display_command);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch (error) {
      setCopyError(error?.message || String(error));
      setTimeout(() => setCopyError(null), 4000);
    }
  };

  const resumeInTerminal = async () => {
    if (resuming) return;
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
            <span className="mono" style={{ color: "var(--tx4)" }}>
              {meta.dir}
            </span>
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
          <button
            className="ftool-btn"
            title={tt("browser:session.refresh")}
            disabled={refreshing}
            onClick={onRefresh}
          >
            {refreshing ? <Spinner size={14} /> : <RefreshIcon />}
          </button>
          {canResume && <button
            className="ftool-btn"
            onClick={resumeInTerminal}
            disabled={resuming}
            title={
              resumeError
                ? tt("browser:session.resumeFailed", { error: resumeError })
                : resuming
                  ? tt("browser:session.resumingTerminal")
                  : tt("browser:session.resumeTerminal")
            }
            style={resumeError ? { color: "var(--err)" } : undefined}
          >
            {resuming ? <Spinner size={14} />
              : resumeError ? <WarnIcon size={15} /> : <TerminalIcon />}
          </button>}
          {canResume && <button
            className="ftool-btn"
            onClick={copyResume}
            title={
              copyError
                ? tt("browser:session.copyResumeFailed", { error: copyError })
                : copied
                  ? tt("browser:session.copiedResume")
                  : tt("browser:session.copyResume")
            }
            style={copied ? { color: "var(--ok)" }
              : copyError ? { color: "var(--err)" } : undefined}
          >
            {copied ? <CheckIcon size={15} />
              : copyError ? <WarnIcon size={15} /> : <CopyIcon size={15} />}
          </button>}
          {optActive && (
            <OptimizerWandControl
              optimization={optimization}
              disabled={!data}
              onStart={onStartOptimization}
            />
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
