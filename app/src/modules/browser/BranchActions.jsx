import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { engine } from "../../platform/desktop/client.js";
import { supportsAgentCapability } from "../../shared/contracts/tools.js";
import {
  BranchIcon,
  HandoffIcon,
  Spinner,
  CheckIcon,
  WarnIcon,
} from "../../shared/ui/icons.jsx";

/** One completed round, two consumers of the same engine-validated checkpoint. */
export default function BranchActions({
  meta,
  round,
  disabled,
  onResumeElsewhere,
  onForkCreated,
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(null);
  const [feedback, setFeedback] = useState(null);
  const inFlight = useRef(false);
  const forkRequest = useRef(null);
  const execute = async (kind) => {
    if (inFlight.current || disabled) return;
    inFlight.current = true;
    setBusy(kind);
    setFeedback(null);
    try {
      const point = await engine("branch_point", {
        tool: meta.tool,
        ref: meta.ref,
        turn_locator: round.locator,
      });
      if (kind === "resume") {
        const result = await onResumeElsewhere(meta, point.through);
        if (!result?.copied)
          throw new Error(result?.error || t("browser:branch.copyFailed"));
        setFeedback({
          kind,
          warning: result.noSkill,
          text: t(
            result.noSkill ? "browser:branch.noSkill" : "browser:branch.copied",
          ),
        });
      } else {
        // Retain the same id on retry: a lost response must not mint another native session.
        forkRequest.current ??= {
          id: crypto.randomUUID(),
          through: point.through,
        };
        if (forkRequest.current.through !== point.through)
          throw new Error(t("browser:branch.changed"));
        const result = await engine("session_fork", {
          tool: meta.tool,
          ref: meta.ref,
          through: point.through,
          request_id: forkRequest.current.id,
        });
        setFeedback({
          kind,
          text: t("browser:branch.created", { id: result.id }),
        });
        await onForkCreated?.(result);
      }
    } catch (error) {
      setFeedback({ kind, error: true, text: error.message });
    } finally {
      inFlight.current = false;
      setBusy(null);
    }
  };
  const actionButton = (kind, Icon) => {
    const status = feedback?.kind === kind ? feedback : null;
    const label = t(`browser:branch.${kind}`);
    return (
      <button
        type="button"
        className="ficon-btn"
        aria-label={label}
        title={status?.text || label}
        disabled={disabled || !!busy}
        onClick={() => execute(kind)}
      >
        {busy === kind ? (
          <Spinner size={13} />
        ) : status?.error || status?.warning ? (
          <WarnIcon />
        ) : status ? (
          <CheckIcon />
        ) : (
          <Icon size={13} />
        )}
      </button>
    );
  };
  return (
    <>
      {supportsAgentCapability(meta.tool, "fork") &&
        actionButton("fork", BranchIcon)}
      {onResumeElsewhere && actionButton("resume", HandoffIcon)}
      {feedback && (
        <span
          role={feedback.error ? "alert" : "status"}
          style={{
            position: "absolute",
            width: 1,
            height: 1,
            padding: 0,
            overflow: "hidden",
            clipPath: "inset(50%)",
            whiteSpace: "nowrap",
          }}
        >
          {feedback.text}
        </span>
      )}
    </>
  );
}
