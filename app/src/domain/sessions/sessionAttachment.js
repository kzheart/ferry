const SCHEME = "ferry-session://";

export function sessionAttachment(session) {
  const tool = String(session?.tool || "").trim();
  const sessionId = String(session?.id || session?.session_id || "").trim();
  if (!tool || tool.length > 32 || !sessionId || sessionId.length > 512
      || /[\0\r\n]/.test(sessionId)) return null;
  return { tool, sessionId, title: String(session?.title || sessionId).slice(0, 200) };
}

export const sessionAttachmentKey = attachment =>
  `${attachment.tool}\u0000${attachment.sessionId}`;

export function sessionIdentity(session) {
  const attachment = sessionAttachment(session);
  return attachment ? sessionAttachmentKey(attachment) : "";
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
  return `${SCHEME}${encodeURIComponent(attachment.tool)}/${encodeURIComponent(attachment.sessionId)}?${query}`;
}

export function parseSessionAttachments(text) {
  if (typeof text !== "string" || !text.includes(SCHEME)) return [];
  const refs = text.match(/ferry-session:\/\/[^\s<>"']+/g) || [];
  return refs.flatMap(ref => {
    try {
      const url = new URL(ref);
      const tool = decodeURIComponent(url.hostname);
      const sessionId = decodeURIComponent(url.pathname.slice(1));
      const attachment = sessionAttachment({
        tool,
        id: sessionId,
        title: url.searchParams.get("title") || sessionId,
      });
      return attachment ? [attachment] : [];
    } catch {
      return [];
    }
  });
}

export function buildSessionPrompt(text, attachments) {
  if (!attachments.length) return text;
  const sessions = attachments.map(({ tool, sessionId }) => ({
    tool,
    session_id: sessionId,
  }));
  return `<ferry_session_refs>${JSON.stringify({ sessions })}</ferry_session_refs>\n\n${text}`;
}

export function sessionDisplayText(text, attachments) {
  const labels = attachments.map(item => `@「${item.title}」`).join(" ");
  return [labels, text].filter(Boolean).join("\n");
}
