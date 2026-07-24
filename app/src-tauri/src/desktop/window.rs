/// 标题栏高度(与前端 App.jsx 里的 44px 保持一致),红绿灯左边距。
#[cfg(target_os = "macos")]
const TITLEBAR_HEIGHT: f64 = 44.0;
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_X: f64 = 14.0;
/// macOS 标准红绿灯间距,仅在无法从现有按钮推算时兜底。
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_SPACING: f64 = 20.0;
/// 浮点比较容差,避免"已就位"被判成需要重摆而在通知回调里空转。
#[cfg(target_os = "macos")]
const EPSILON: f64 = 0.5;

/// macOS 在窗口显示/聚焦/缩放时会把红绿灯重置回默认位置,
/// 因此不用 tauri.conf 的 trafficLightPosition,而是自己摆:
/// 把标题栏容器撑到 TITLEBAR_HEIGHT 高,再把三个按钮垂直居中。
///
/// 本函数幂等:已就位时不写任何 frame。这既是性能考虑,
/// 更是防递归——它会被 frame 变更通知回调调用,而写 frame 会同步再发通知。
#[cfg(target_os = "macos")]
fn align_traffic_lights(ns_window: &objc2_app_kit::NSWindow) {
    use objc2_app_kit::NSWindowButton;
    unsafe {
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

        let target_y = ns_window.frame().size.height - TITLEBAR_HEIGHT;
        let mut rect = container.frame();
        if (rect.size.height - TITLEBAR_HEIGHT).abs() > EPSILON
            || (rect.origin.y - target_y).abs() > EPSILON
        {
            rect.size.height = TITLEBAR_HEIGHT;
            rect.origin.y = target_y;
            container.setFrame(rect);
        }

        let spacing = mini.frame().origin.x - close.frame().origin.x;
        let spacing = if spacing > EPSILON {
            spacing
        } else {
            TRAFFIC_LIGHT_SPACING
        };
        let mut buttons = vec![close, mini];
        buttons.extend(zoom);
        for (i, button) in buttons.iter().enumerate() {
            let frame = button.frame();
            let target_x = TRAFFIC_LIGHT_X + i as f64 * spacing;
            let target_y = (TITLEBAR_HEIGHT - frame.size.height) / 2.0;
            if (frame.origin.x - target_x).abs() > EPSILON
                || (frame.origin.y - target_y).abs() > EPSILON
            {
                button.setFrameOrigin(objc2_foundation::NSPoint::new(target_x, target_y));
            }
        }
    }
}

/// 定时重摆治标不治本:纠正发生在窗口可见之后,肉眼会看到红绿灯跳。
/// 改为订阅标题栏容器和三个按钮的 frame 变更通知,AppKit 一重排就在
/// 同一轮 runloop 内同步摆回去,屏幕上不会刷出中间态。
#[cfg(target_os = "macos")]
pub(crate) fn install(window: &tauri::Window) {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSViewFrameDidChangeNotification, NSWindow, NSWindowButton};
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use std::ptr::NonNull;

    let Ok(ptr) = window.ns_window() else { return };
    unsafe {
        let ns_window = &*(ptr as *const NSWindow);

        let mut observed: Vec<Retained<NSView>> = Vec::new();
        for button in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            let Some(button) = ns_window.standardWindowButton(button) else {
                continue;
            };
            if observed.is_empty() {
                // 容器只需订阅一次,三个按钮共用同一个
                if let Some(container) = button.superview().and_then(|v| v.superview()) {
                    observed.push(container);
                }
            }
            // NSButton -> NSControl -> NSView
            observed.push(Retained::into_super(Retained::into_super(button)));
        }

        let center = NSNotificationCenter::defaultCenter();
        for view in &observed {
            view.setPostsFrameChangedNotifications(true);
            let block = block2::RcBlock::new(move |notification: NonNull<NSNotification>| {
                let Some(object) = notification.as_ref().object() else {
                    return;
                };
                let Some(view) = object.downcast_ref::<NSView>() else {
                    return;
                };
                let Some(window) = view.window() else { return };
                align_traffic_lights(&window);
            });
            // observer token 需常驻:窗口与进程同生命周期,退出时统一回收
            std::mem::forget(center.addObserverForName_object_queue_usingBlock(
                Some(NSViewFrameDidChangeNotification),
                Some(view),
                None,
                &block,
            ));
        }

        align_traffic_lights(ns_window);
    }
}

pub(crate) fn handle_window_event(_window: &tauri::Window, _event: &tauri::WindowEvent) {
    #[cfg(target_os = "macos")]
    if matches!(
        _event,
        tauri::WindowEvent::Resized(_)
            | tauri::WindowEvent::Moved(_)
            | tauri::WindowEvent::Focused(_)
            | tauri::WindowEvent::ThemeChanged(_)
            | tauri::WindowEvent::ScaleFactorChanged { .. }
    ) {
        if let Ok(ptr) = _window.ns_window() {
            unsafe { align_traffic_lights(&*(ptr as *const objc2_app_kit::NSWindow)) };
        }
    }
}
