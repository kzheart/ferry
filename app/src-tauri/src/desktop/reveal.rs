//! 在系统文件管理器中打开路径：目录直接打开，文件则在父目录中选中。

#[tauri::command]
pub(crate) async fn reveal_path(path: String) -> Result<(), String> {
    let target = std::path::Path::new(&path);
    if !target.exists() {
        return Err("路径不存在".to_string());
    }
    super::platform::reveal_path(target)
}
