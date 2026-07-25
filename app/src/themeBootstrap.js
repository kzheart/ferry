// CSS/React 就绪前恢复主题，避免首帧白闪。
try {
  const settings = JSON.parse(
    localStorage.getItem("ferry-settings") || "{}",
  );
  const dark = settings.theme === "dark"
    || (
      settings.theme === "system"
      && matchMedia("(prefers-color-scheme: dark)").matches
    );
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  document.documentElement.style.background = dark
    ? "#141416"
    : "#FBFCFD";
  // 窗口外观(毛玻璃材质)也要赶在首帧前同步:应用主题与系统深浅色不一致时,
  // 等 React 挂载后再 setTheme 会让侧栏先以错误材质渲染一下再跳变
  const theme = settings.theme ?? "light";
  if ("__TAURI_INTERNALS__" in window && theme !== "system") {
    import("@tauri-apps/api/window")
      .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(theme))
      .catch(() => {});
  }
} catch {
  // 无有效缓存时保持默认浅色主题。
}
