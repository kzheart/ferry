import { useTranslation } from "react-i18next";
import Markdown from "../../shared/ui/Markdown.jsx";
import { Spinner } from "../../shared/ui/icons.jsx";
import { AgentToolRow } from "./AgentToolTrace.jsx";
import { ApprovalCard, WorkflowCard } from "./AgentWorkflowCards.jsx";

export function AgentChatItem({ item, sessionId, ferry }) {
  const { t } = useTranslation();
  if (item.kind === "user") {
    return (
      <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 3 }}>
        {item.sub && (
          <span style={{ fontSize: 10.5, color: "var(--tx5)", paddingRight: 6 }}>
            {t(item.sub === "steer"
              ? "askferry:chat.steered"
              : "askferry:chat.followedUp")}
          </span>
        )}
        <div className="chat-user selectable">{item.text}</div>
      </div>
    );
  }
  if (item.kind === "assistant") {
    return (
      <div className="selectable">
        <Markdown text={item.text} />
        {item.streaming && <div style={{ marginTop: 6 }}><Spinner size={12} /></div>}
      </div>
    );
  }
  if (item.kind === "tool") return <AgentToolRow item={item} />;
  if (item.kind === "workflow") return <WorkflowCard item={item} />;
  if (item.kind === "approval") {
    return (
      <ApprovalCard item={item}
        onApprove={() => ferry.approve(sessionId, item)}
        onDismiss={() => ferry.dismiss(sessionId, item)} />
    );
  }
  if (item.kind === "status") {
    const status = {
      "run.failed": [
        "var(--err-text)",
        t("askferry:chat.runFailed", { message: item.message || "" }),
      ],
      "run.cancelled": ["var(--tx5)", t("askferry:chat.runCancelled")],
      "run.interrupted": ["var(--warn-text)", t("askferry:chat.runInterrupted")],
    };
    const [color, label] = status[item.type] || ["var(--tx5)", item.type];
    return (
      <div style={{ fontSize: 11.5, color, textAlign: "center", padding: "2px 0" }}>
        {label}
      </div>
    );
  }
  return null;
}
