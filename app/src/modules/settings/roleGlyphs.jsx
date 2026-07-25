// 角色页专用的 16×16 线性图标:只服务这一页的工具卡与分组标题,不进通用图标库。
export const TOOL_GLYPH = {
  session_search: '<circle cx="7" cy="7" r="4.6" fill="none" stroke="currentColor" stroke-width="1.5"/><line x1="10.4" y1="10.4" x2="14" y2="14" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>',
  session_read: '<path d="M3.4 2.4h5.4l3.8 3.8v7.4H3.4z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="M8.6 2.4v3.8h3.8M5.6 9h4.8M5.6 11.4h3.2" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>',
  usage: '<path d="M2.6 13.4h10.8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><rect x="3.6" y="8" width="2.4" height="3.6" rx=".7" fill="currentColor"/><rect x="6.9" y="5.2" width="2.4" height="6.4" rx=".7" fill="currentColor"/><rect x="10.2" y="2.8" width="2.4" height="8.8" rx=".7" fill="currentColor"/>',
  migrate: '<path d="M9.6 3.2h3a1.2 1.2 0 0 1 1.2 1.2v7.2a1.2 1.2 0 0 1-1.2 1.2h-3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/><path d="M1.8 8h8M7 4.8 10.2 8 7 11.2" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
  session_edit: '<path d="M11.1 2.6a1.5 1.5 0 0 1 2.1 2.1L6 11.9l-2.9.8.8-2.9z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="M2.6 14.2h10.8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>',
  bash: '<rect x="1.8" y="2.8" width="12.4" height="10.4" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M4.6 6.2 6.8 8l-2.2 1.8M8.4 10.2h3" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>',
};

export const GROUP_GLYPH = {
  identity: '<circle cx="8" cy="5.4" r="2.7" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M3.2 13.6c.5-2.7 2.1-4 4.8-4s4.3 1.3 4.8 4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>',
  persona: '<path d="M3 3.4h10M3 7h10M3 10.6h6.4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>',
  capability: '<path d="M6.4 2.6h3.2v2a1.4 1.4 0 0 0 2.8 0h1.2v3.2h-2a1.4 1.4 0 0 0 0 2.8v3.2H2.4V10.6a1.4 1.4 0 0 0 0-2.8V4.6h4z" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/>',
  skill: '<path d="M8 1.8 9.9 5.7l4.3.6-3.1 3 .7 4.3L8 11.6l-3.8 2 .7-4.3-3.1-3 4.3-.6z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>',
  model: '<path d="M8 2 14 5.4 8 8.8 2 5.4Z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="m2 8.6 6 3.4 6-3.4M2 11.6 8 15l6-3.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>',
  security: '<path d="M8 1.8 13.4 4v4.2c0 3-2.2 5.2-5.4 6-3.2-.8-5.4-3-5.4-6V4z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="m5.6 7.9 1.8 1.8 3.2-3.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>',
};

export const EXPORT_GLYPH = '<path d="M8 2.6v6.6M5.5 6.9 8 9.4l2.5-2.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/><path d="M2.9 10.8v1.5a1.2 1.2 0 0 0 1.2 1.2h7.8a1.2 1.2 0 0 0 1.2-1.2v-1.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>';

export const UNDO_GLYPH = '<path d="M6 9.3 2.7 6 6 2.7" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/><path d="M2.7 6h7a3.7 3.7 0 0 1 0 7.4H7.4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>';

export const INFO_GLYPH = '<circle cx="8" cy="8" r="6.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M8 7.2v4M8 4.9v.9" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>';

export const glyph = (markup, size = 13) => (
  <svg viewBox="0 0 16 16" aria-hidden style={{ width: size, height: size, flex: "none" }}
    dangerouslySetInnerHTML={{ __html: markup }} />
);
