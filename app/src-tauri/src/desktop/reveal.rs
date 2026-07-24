//! 在系统文件管理器中定位会话文件。

#[tauri::command]
pub(crate) async fn reveal_path(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).exists() {
        return Err("文件不存在".to_string());
    }
    super::platform::reveal_path(std::path::Path::new(&path))
}
