//! 角色配置的导入导出。
//!
//! 只暴露"选一个 JSON 文件读/写"这一件事:路径由系统对话框产生、读写发生在 Rust 侧,
//! webview 拿不到通用文件系统能力,也无法指定任意路径。

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

/// 角色配置是纯文本 JSON,超过这个体积一定不是我们导出的文件。
const MAX_ROLE_FILE_BYTES: u64 = 2 * 1024 * 1024;

fn json_dialog(app: &AppHandle) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
    app.dialog()
        .file()
        .add_filter("Ferry 角色配置", &["json"])
}

/// 弹出保存对话框并写入角色配置;返回落盘路径,用户取消时返回 None。
#[tauri::command]
pub(crate) async fn export_roles_file(
    app: AppHandle,
    file_name: String,
    contents: String,
) -> Result<Option<String>, String> {
    let Some(target) = json_dialog(&app)
        .set_file_name(file_name)
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = target.into_path().map_err(|error| error.to_string())?;
    std::fs::write(&path, contents).map_err(|error| error.to_string())?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// 弹出选择对话框并读回角色配置文本;用户取消时返回 None。
#[tauri::command]
pub(crate) async fn import_roles_file(app: AppHandle) -> Result<Option<String>, String> {
    let Some(source) = json_dialog(&app).blocking_pick_file() else {
        return Ok(None);
    };
    let path = source.into_path().map_err(|error| error.to_string())?;
    let size = std::fs::metadata(&path)
        .map_err(|error| error.to_string())?
        .len();
    if size > MAX_ROLE_FILE_BYTES {
        return Err("角色配置文件过大".to_string());
    }
    std::fs::read_to_string(&path).map(Some).map_err(|error| error.to_string())
}
