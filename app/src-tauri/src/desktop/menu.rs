//! macOS 原生菜单栏:替换 Tauri 默认菜单,自定义项通过 "menu" 事件转发给前端处理。

use tauri::menu::{AboutMetadataBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter};

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let about = AboutMetadataBuilder::new().name(Some("Ferry")).build();

    let app_menu = SubmenuBuilder::new(app, "Ferry")
        .about_with_text("关于 Ferry", Some(about))
        .separator()
        .item(
            &MenuItemBuilder::with_id("settings", "设置…")
                .accelerator("Cmd+,")
                .build(app)?,
        )
        .separator()
        .services_with_text("服务")
        .separator()
        .hide_with_text("隐藏 Ferry")
        .hide_others_with_text("隐藏其他")
        .show_all_with_text("全部显示")
        .separator()
        .quit_with_text("退出 Ferry")
        .build()?;

    // 撤销/拷贝/粘贴走系统响应链,输入框内可用
    let edit = SubmenuBuilder::new(app, "编辑")
        .undo_with_text("撤销")
        .redo_with_text("重做")
        .separator()
        .cut_with_text("剪切")
        .copy_with_text("拷贝")
        .paste_with_text("粘贴")
        .select_all_with_text("全选")
        .build()?;

    let view = SubmenuBuilder::new(app, "显示")
        .item(
            &MenuItemBuilder::with_id("toggle-sidebar", "隐藏/显示边栏")
                .accelerator("Cmd+B")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("rescan", "重新扫描会话")
                .accelerator("Cmd+R")
                .build(app)?,
        )
        .separator()
        .fullscreen_with_text("进入/退出全屏")
        .build()?;

    let window = SubmenuBuilder::new(app, "窗口")
        .minimize_with_text("最小化")
        .maximize_with_text("缩放")
        .separator()
        .close_window_with_text("关闭窗口")
        .build()?;

    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &edit, &view, &window])
        .build()?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let _ = app.emit("menu", event.id().0.clone());
    });
    Ok(())
}
