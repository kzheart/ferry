/// 标题栏高度(与前端 App.jsx 里的 44px 保持一致),红绿灯左边距。
#[cfg(target_os = "macos")]
const TITLEBAR_HEIGHT: f64 = 44.0;
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_X: f64 = 14.0;

/// macOS 在窗口显示/聚焦/缩放时会把红绿灯重置回默认位置,
/// 因此不用 tauri.conf 的 trafficLightPosition,而是在窗口事件里反复重摆:
/// 把标题栏容器撑到 TITLEBAR_HEIGHT 高,再把三个按钮垂直居中。
#[cfg(target_os = "macos")]
fn align_traffic_lights(window: &tauri::Window) {
    use objc2_app_kit::{NSWindow, NSWindowButton};
    let Ok(ptr) = window.ns_window() else { return };
    unsafe {
        let ns_window = &*(ptr as *const NSWindow);
        let Some(close) = ns_window.standardWindowButton(NSWindowButton::CloseButton) else {
            return;
        };
        let Some(mini) = ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
            return;
        };
        let zoom = ns_window.standardWindowButton(NSWindowButton::ZoomButton);
        let Some(container) = close.superview().and_then(|v| v.superview()) else {
            return;
        };

        let mut rect = container.frame();
        rect.size.height = TITLEBAR_HEIGHT;
        rect.origin.y = ns_window.frame().size.height - TITLEBAR_HEIGHT;
        container.setFrame(rect);

        let spacing = mini.frame().origin.x - close.frame().origin.x;
        let mut buttons = vec![close, mini];
        buttons.extend(zoom);
        for (i, button) in buttons.iter().enumerate() {
            let mut frame = button.frame();
            frame.origin.x = TRAFFIC_LIGHT_X + i as f64 * spacing;
            frame.origin.y = (TITLEBAR_HEIGHT - frame.size.height) / 2.0;
            button.setFrameOrigin(frame.origin);
        }
    }
}

pub(crate) fn handle_window_event(_window: &tauri::Window, _event: &tauri::WindowEvent) {
    #[cfg(target_os = "macos")]
    if matches!(
        _event,
        tauri::WindowEvent::Resized(_)
            | tauri::WindowEvent::Focused(_)
            | tauri::WindowEvent::ThemeChanged(_)
    ) {
        align_traffic_lights(_window);
    }
}
