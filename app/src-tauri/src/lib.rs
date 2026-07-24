//! Tauri 壳不含会话格式知识，只转发引擎 RPC 和启动受限的接续命令。

mod agent;
mod contracts;
#[cfg(target_os = "macos")]
mod menu;
mod operation_commands;
mod operation_input;
mod operation_request;
mod operation_validation;
mod platform;
mod reveal;
mod sidecar;
mod sidecar_policy;
mod terminal;
mod window;

pub fn run() {
    // 必须在 spawn 任何引擎进程之前修复 PATH,子进程只在 fork 时继承一次环境。
    let _ = fix_path_env::fix();
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            use tauri::Manager;
            // 引擎预热与 webview 启动并行,首个 RPC 无需再等冷启动
            if let Ok(resource_dir) = app.path().resource_dir() {
                sidecar::warm_up(resource_dir.clone());
                agent::warm_up(app.handle().clone(), resource_dir);
            }
            #[cfg(target_os = "macos")]
            {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = window_vibrancy::apply_vibrancy(
                        &win,
                        window_vibrancy::NSVisualEffectMaterial::Sidebar,
                        None,
                        None,
                    );
                }
                menu::install(app.handle())?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            sidecar::engine_rpc,
            operation_commands::operation_plan,
            operation_commands::operation_apply,
            operation_commands::operation_status,
            operation_commands::operation_cancel,
            agent::agent_command,
            terminal::open_terminal,
            reveal::reveal_path
        ])
        .on_window_event(window::handle_window_event)
        .run(tauri::generate_context!())
        .expect("Ferry 启动失败");
}
