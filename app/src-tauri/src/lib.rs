//! Tauri 壳不含会话格式知识，只转发引擎 RPC 和启动受限的接续命令。

mod contracts;
mod desktop;
mod engine;
mod operations;
mod process;
mod runtime;

pub fn run() {
    // 必须在 spawn 任何引擎进程之前修复 PATH,子进程只在 fork 时继承一次环境。
    let _ = fix_path_env::fix();
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            use tauri::Manager;
            // 引擎预热与 webview 启动并行,首个 RPC 无需再等冷启动
            if let Ok(resource_dir) = app.path().resource_dir() {
                engine::warm_up(app.handle().clone(), resource_dir.clone());
                runtime::warm_up(app.handle().clone(), resource_dir);
            }
            // 已装过的 CLI / Skill 随本 App 打包内容静默对齐;未装过的不主动装。
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    desktop::integration::sync_managed_integrations(&handle);
                });
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
                    desktop::window::install(&win.as_ref().window());
                }
                desktop::menu::install(app.handle())?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine::engine_rpc,
            operations::operation_plan,
            operations::operation_apply,
            operations::operation_status,
            operations::operation_cancel,
            runtime::agent_command,
            runtime::choice::choice_respond,
            runtime::bash::bash_apply,
            desktop::terminal::open_terminal,
            desktop::reveal::reveal_path,
            desktop::role_file::export_roles_file,
            desktop::role_file::import_roles_file,
            desktop::skill_dir::pick_skill_directory,
            desktop::integration::integration_status,
            desktop::integration::cli_install,
            desktop::integration::cli_uninstall,
            desktop::integration::skill_install,
            desktop::integration::skill_uninstall,
            desktop::integration::engine_service_status,
            desktop::features::features_list,
            desktop::features::feature_set,
            desktop::integration::get_engine_share,
            desktop::integration::set_engine_share,
            desktop::integration::engine_daemon_stop
        ])
        .on_window_event(desktop::window::handle_window_event)
        .run(tauri::generate_context!())
        .expect("Ferry 启动失败");
}
