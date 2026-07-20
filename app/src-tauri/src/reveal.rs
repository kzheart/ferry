//! 在系统文件管理器中定位会话文件。

#[tauri::command]
pub(crate) async fn reveal_path(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).exists() {
        return Err("文件不存在".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("open")
            .args(["-R", &path])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    Err("当前平台尚未支持在文件管理器中定位".to_string())
}
