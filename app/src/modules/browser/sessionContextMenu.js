import {
  openTerminal,
  revealPath,
  writeClipboardText,
} from "../../platform/desktop/client.js";
import {
  TOOLS,
  resumeDescriptor,
} from "../../shared/contracts/tools.js";
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
  setBatchDelete,
  setMultiIds,
  setAgentAttachments,
  setView,
  setMenu,
  setToast,
  select,
  setMigration,
  settings,
  t,
  askDelete,
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
        label: t("app:ctx.deleteN", { n: multipleSessions.length }),
        danger: true,
        onClick: () => setBatchDelete(multipleSessions),
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

  return [
    { label: t("app:ctx.addToAgent"), onClick: addToAgent },
    {
      label: t("app:ctx.resumeTerminal"),
      hint: "↩",
      onClick: () => resumeDescriptor(session.tool, sessionRef(session))
        .then(launch => openTerminal(launch, settings.terminalApp))
        .catch(() => {}),
    },
    ...(TOOLS.includes(session.tool) ? [{
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
    {
      label: t("app:ctx.copyResume"),
      onClick: () => resumeDescriptor(session.tool, sessionRef(session))
        .then(descriptor => writeClipboardText(descriptor.display_command))
        .catch(() => {}),
    },
    {
      label: t("app:ctx.revealInFinder"),
      disabled: !session.path,
      disabledHint: t("app:ctx.noSessionFile"),
      onClick: () => revealPath(session.path).catch(() => {}),
    },
    { sep: true },
    {
      label: t("app:ctx.deleteSession"),
      hint: "⌫",
      danger: true,
      onClick: () => askDelete(session),
    },
  ];
}
