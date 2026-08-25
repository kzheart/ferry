import {
  openTerminal,
  revealPath,
  writeClipboardText,
} from "../../platform/desktop/client.js";
import {
  TOOLS,
  supportsAgentCapability,
  supportsSessionResumeCli,
  resumeDescriptor,
} from "../../shared/contracts/tools.js";
import { filterByFeatures } from "../../shared/capabilities/features.jsx";
import {
  addSessionAttachment,
  serializeSessionAttachment,
} from "./sessionAttachment.js";
import { sessionIdentity } from "./sessionAttachment.js";
import { sessionRef } from "./sessionModel.js";

export function createSessionContextMenu({
  menu,
  sessionsByKey,
  selectedId,
  multiIds,
  metaFor,
  updateMetadata,
  setTagSelection,
  setRename,
  setMultiIds,
  setAgentAttachments,
  setView,
  setMenu,
  setToast,
  select,
  setMigration,
  onResumeElsewhere,
  settings,
  isFeatureEnabled,
  t,
}) {
  const session = menu ? sessionsByKey[menu.key] : null;
  const metadata = session ? metaFor(session) : {};
  const multipleSessions = multiIds
    .map(key => sessionsByKey[key])
    .filter(Boolean);

  if (menu?.multi) {
    return [
      {
        label: t("app:ctx.addTags"),
        onClick: () => setTagSelection({ sessions: multipleSessions, batch: true }),
      },
      { sep: true },
      {
        label: t("app:ctx.cancelMulti"),
        onClick: () => setMultiIds([]),
      },
    ];
  }

  if (!session) return null;

  const addToAgent = () => {
    setAgentAttachments(attachments =>
      addSessionAttachment(attachments, session));
    setView("askferry");
    setMenu(null);
  };
  const copySessionReference = () => {
    writeClipboardText(serializeSessionAttachment(session))
      .then(() => {
        setToast({
          kind: "ok",
          title: t("app:toast.sessionReferenceCopied"),
          desc: t("app:toast.sessionReferenceCopiedDesc"),
        });
      })
      .catch(() => {});
  };

  // 标了 feature 的项由开关决定出不出现:「加入对话」通往内置 AI 助手
  return filterByFeatures([
    {
      label: t("app:ctx.addToAgent"),
      onClick: addToAgent,
      feature: "builtin-agent",
    },
    ...(supportsAgentCapability(session.tool, "resume") ? [{
      label: t("app:ctx.resumeTerminal"),
      hint: "↩",
      disabled: !supportsSessionResumeCli(session.tool),
      disabledHint: t("app:ctx.resumeCliUnavailable"),
      onClick: () => resumeDescriptor(session.tool, sessionRef(session))
        .then(launch => openTerminal(launch, settings.terminalApp))
        .catch(() => {}),
    }] : []),
    // 续聊到其他 agent:只复制一条 `/ferry-resume <tool> <id>`,不发任何 RPC。
    // 指令内容与粘到哪个 agent 无关,所以只有一条入口,不按目标分列。
    {
      label: t("app:ctx.copyResumeElsewhere"),
      onClick: () => onResumeElsewhere?.(session),
    },
    ...(TOOLS.includes(session.tool)
      && supportsAgentCapability(session.tool, "migration-source") ? [{
      label: t("app:ctx.migrateTo"),
      onClick: () => {
        if (sessionIdentity(session) !== selectedId) {
          select(sessionIdentity(session));
        }
        setMigration({ scope: null });
      },
    }] : []),
    { sep: true },
    {
      label: t("app:ctx.rename"),
      hint: "F2",
      onClick: () => setRename(session),
    },
    {
      label: metadata.pinned ? t("app:ctx.unpin") : t("app:ctx.pin"),
      onClick: () => updateMetadata(session, { pinned: !metadata.pinned }),
    },
    {
      label: t("app:ctx.tags"),
      onClick: () => setTagSelection({ sessions: [session] }),
    },
    { sep: true },
    {
      label: t("app:ctx.copySessionReference"),
      onClick: copySessionReference,
    },
    {
      label: t("app:ctx.copyId"),
      onClick: () => writeClipboardText(session.id).catch(() => {}),
    },
    ...(supportsAgentCapability(session.tool, "resume") ? [{
      label: t("app:ctx.copyResume"),
      disabled: !supportsSessionResumeCli(session.tool),
      disabledHint: t("app:ctx.resumeCliUnavailable"),
      onClick: () => resumeDescriptor(session.tool, sessionRef(session))
        .then(descriptor => writeClipboardText(descriptor.display_command))
        .catch(() => {}),
    }] : []),
    {
      label: t("app:ctx.revealInFinder"),
      disabled: !session.dir,
      disabledHint: t("app:ctx.noProjectDir"),
      onClick: () => revealPath(session.dir).catch(() => {}),
    },
  ], isFeatureEnabled);
}
