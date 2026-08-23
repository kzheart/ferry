import { useTranslation } from "react-i18next";

import Markdown from "../../shared/ui/Markdown.jsx";

// 更新装完重启后的第一屏。正文就是 latest.json 里的 notes,也就是
// CHANGELOG 里这一版的那一节,所以按 Markdown 渲染。
export function UpdateAnnouncement({ announcement, onDismiss, onOpenUpdates }) {
  const { t, i18n } = useTranslation();
  if (!announcement) return null;

  const { from, to, date, notes } = announcement;
  const shownDate = date ? new Date(date) : null;
  const validDate = shownDate && !Number.isNaN(shownDate.valueOf()) ? shownDate : null;

  return (
    <div style={{ position: "absolute", inset: 0, background: "var(--scrim)", display: "flex",
      alignItems: "center", justifyContent: "center", zIndex: 44 }}
      onClick={onDismiss}>
      <div role="dialog" aria-modal="true" aria-label={t("settings:updates.announceTitle", { version: to })}
        onClick={event => event.stopPropagation()}
        style={{ width: 460, maxWidth: "calc(100vw - 40px)", background: "var(--bg)",
          borderRadius: 12, boxShadow: "var(--shadow-sheet)", overflow: "hidden",
          display: "flex", flexDirection: "column", maxHeight: "min(560px, calc(100vh - 80px))" }}>
        <div style={{ padding: "22px 22px 16px", borderBottom: ".5px solid var(--line6)" }}>
          <div style={{ fontSize: 15, fontWeight: 600, color: "var(--tx1)" }}>
            {t("settings:updates.announceTitle", { version: to })}
          </div>
          <div className="mono" style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 6 }}>
            {from && from !== "—" ? `v${from} → v${to}` : `v${to}`}
            {validDate ? ` · ${validDate.toLocaleDateString(i18n.language)}` : ""}
          </div>
        </div>

        {notes ? (
          <div className="fscroll" style={{ flex: 1, minHeight: 0, overflowY: "auto",
            padding: "16px 22px", fontSize: 13 }}>
            <Markdown text={notes} />
          </div>
        ) : (
          <div style={{ padding: "16px 22px", fontSize: 13, color: "var(--tx3b)" }}>
            {t("settings:updates.announceNoNotes")}
          </div>
        )}

        <div style={{ padding: "14px 22px 20px", display: "flex", gap: 10,
          justifyContent: "flex-end", borderTop: ".5px solid var(--line6)" }}>
          <button className="fbtn" style={{ height: 34, fontSize: 13 }}
            onClick={() => { onDismiss(); onOpenUpdates(); }}>
            {t("settings:updates.announceOpenSettings")}
          </button>
          <button className="fbtn-primary" autoFocus
            style={{ height: 34, padding: "0 16px", fontSize: 13 }} onClick={onDismiss}>
            {t("settings:updates.announceDismiss")}
          </button>
        </div>
      </div>
    </div>
  );
}
