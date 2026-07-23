use std::path::Path;

/// Windows 实现预留在平台层。会话、审批和 Tauri command 不应感知其差异。
pub(super) fn reveal_path(_path: &Path) -> Result<(), String> {
    Err("Windows 文件管理器定位尚未实现".to_owned())
}
