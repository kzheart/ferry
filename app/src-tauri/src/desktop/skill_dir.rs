//! 技能目录选择器。
//!
//! 只暴露"让用户挑一个文件夹"这一件事:路径由系统对话框产生,webview 既不能指定
//! 任意路径,也拿不到通用文件系统能力——真正的读写都在 ferry-runtime 里做。

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

/// 弹出目录选择对话框;返回所选目录的绝对路径,用户取消时返回 None。
#[tauri::command]
pub(crate) async fn pick_skill_directory(app: AppHandle) -> Result<Option<String>, String> {
    let Some(target) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path = target.into_path().map_err(|error| error.to_string())?;
    if !path.is_dir() {
        return Err("所选路径不是目录".to_string());
    }
    Ok(Some(path.to_string_lossy().into_owned()))
}
