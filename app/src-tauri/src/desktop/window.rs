//! macOS 红绿灯位置:交给 AppKit 默认布局,不再手工 setFrame / setFrameOrigin。
//!
//! 手工改容器高度(32→44)或按钮 x(9→14)会在放大/全屏时与 AppKit 对打:
//! 轻则红绿灯闪动,重则 stack overflow。前端 44px 拖拽区可继续用;
//! 红绿灯落在原生 ~32pt 条带内,这是 macOS overlay 标题栏的常态。

pub(crate) fn install(_window: &tauri::Window) {}

pub(crate) fn handle_window_event(_window: &tauri::Window, _event: &tauri::WindowEvent) {}
