import { useTranslation } from "react-i18next";

import { UpdateIcon, UpdateRing } from "../../shared/ui/icons.jsx";

// 导航栏「设置」行行尾的更新入口。只在有事可做时出现——没更新时整行保持干净,
// 留一个常灰的图标位只会让人以为它坏了。
//
// 用 span + role 而不是 <button>:承载它的 NavRow 本身就是个 button,
// 按钮套按钮是非法嵌套,点击派发也不可靠。
export function UpdateBadge({ phase, version, progress, onStart }) {
  const { t } = useTranslation();
  const ready = phase === "available";
  if (!ready && phase !== "downloading" && phase !== "installing") return null;

  const percent = progress == null ? null : Math.round(progress * 100);
  const title = ready
    ? t("settings:updates.badgeAvailable", { version })
    : phase === "downloading"
      ? (percent == null
        ? t("settings:updates.badgeDownloading", { version })
        : t("settings:updates.badgeDownloadingPercent", { version, percent }))
      : t("settings:updates.badgeInstalling");

  // 图标压在整行的点击区上,不 stop 就会连带把设置页打开
  const start = event => {
    event.stopPropagation();
    if (ready) onStart();
  };

  return (
    <span role="button" tabIndex={ready ? 0 : -1} title={title} aria-label={title}
      aria-disabled={ready ? undefined : "true"}
      onClick={start}
      onKeyDown={event => {
        if (event.key !== "Enter" && event.key !== " ") return;
        event.preventDefault();
        start(event);
      }}
      style={{ flex: "none", width: 22, height: 22, marginRight: -3, display: "flex",
        alignItems: "center", justifyContent: "center", borderRadius: 6,
        color: "var(--accent)", cursor: ready ? "pointer" : "default" }}>
      {phase === "downloading"
        ? <UpdateRing size={16} progress={progress} />
        : phase === "installing"
          ? <UpdateRing size={16} progress={null} />
          : <UpdateIcon size={15} />}
    </span>
  );
}
