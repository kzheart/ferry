// macOS overlay 标题栏要把红绿灯区域让出来；Windows / Linux 用系统标题栏，
// 侧栏顶部不再留 44px 空带。
export const OVERLAY_TITLEBAR =
  typeof navigator !== "undefined" &&
  /Mac/i.test(`${navigator.platform || ""} ${navigator.userAgent || ""}`);

export const TITLEBAR_INSET = OVERLAY_TITLEBAR ? 44 : 8;
