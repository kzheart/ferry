//! Tauri 壳不含会话格式知识，只转发引擎 RPC 和启动受限的接续命令。

mod reveal;
mod sidecar;
mod terminal;
mod window;

pub fn run() {
    // 必须在 spawn 任何引擎进程之前修复 PATH,子进程只在 fork 时继承一次环境。
    let _ = fix_path_env::fix();
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            sidecar::engine_rpc,
            terminal::open_terminal,
            reveal::reveal_path
        ])
        .on_window_event(window::handle_window_event)
        .run(tauri::generate_context!())
        .expect("Ferry 启动失败");
}
