import { isOpaqueSessionRef } from "../../api/contract/generated/session-ref.js";

const SCHEME = "ferry-session://";

export function sessionAttachment(session) {
  const tool = String(session?.tool || "").trim();
  const ref = String(session?.ref || "").trim();
  if (!tool || tool.length > 32 || !isOpaqueSessionRef(ref)) return null;
  return { tool, ref, title: String(session?.title || ref).slice(0, 200) };
}

export const sessionAttachmentKey = attachment =>
  `${attachment.tool}\u0000${attachment.ref}`;

export function sessionIdentity(session) {
  const tool = String(session?.tool || "").trim();
  const sessionId = String(session?.id || session?.session_id || "").trim();
  if (!tool || tool.length > 32 || !sessionId || sessionId.length > 512
      || /[\0\r\n]/.test(sessionId)) return "";
  return `${tool}\u0000${sessionId}`;
}

export function addSessionAttachment(list, candidate) {
  const attachment = sessionAttachment(candidate);
  if (!attachment) return list;
  const key = sessionAttachmentKey(attachment);
  return list.some(item => sessionAttachmentKey(item) === key)
    ? list
    : [...list, attachment];
}

export function serializeSessionAttachment(candidate) {
  const attachment = sessionAttachment(candidate);
  if (!attachment) return "";
  const query = new URLSearchParams({ title: attachment.title });
  return `${SCHEME}${encodeURIComponent(attachment.tool)}/${encodeURIComponent(attachment.ref)}?${query}`;
}

export function parseSessionAttachments(text) {
  if (typeof text !== "string" || !text.includes(SCHEME)) return [];
  const refs = text.match(/ferry-session:\/\/[^\s<>"']+/g) || [];
  return refs.flatMap(ref => {
    try {
      const url = new URL(ref);
      const tool = decodeURIComponent(url.hostname);
      const sessionRef = decodeURIComponent(url.pathname.slice(1));
      const attachment = sessionAttachment({
        tool,
        ref: sessionRef,
        title: url.searchParams.get("title") || sessionRef,
      });
      return attachment ? [attachment] : [];
    } catch {
      return [];
    }
  });
}

export function buildSessionPrompt(text, attachments) {
  if (!attachments.length) return text;
  const sessions = attachments.map(({ tool, ref }) => ({
    tool,
    ref,
  }));
  return `<ferry_session_refs>${JSON.stringify({ sessions })}</ferry_session_refs>\n\n${text}`;
}

export function sessionDisplayText(text, attachments) {
  const labels = attachments.map(item => `@「${item.title}」`).join(" ");
  return [labels, text].filter(Boolean).join("\n");
}
