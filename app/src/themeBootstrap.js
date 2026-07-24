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
} catch {
  // 无有效缓存时保持默认浅色主题。
}
