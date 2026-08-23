/// 标题栏高度(与前端 App.jsx 里的 44px 保持一致),红绿灯左边距。
#[cfg(target_os = "macos")]
const TITLEBAR_HEIGHT: f64 = 44.0;
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_X: f64 = 14.0;
/// macOS 标准红绿灯间距,仅在无法从现有按钮推算时兜底。
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_SPACING: f64 = 23.0;
/// 浮点比较容差,避免"已就位"被判成需要重摆而在通知回调里空转。
#[cfg(target_os = "macos")]
const EPSILON: f64 = 0.5;

#[cfg(target_os = "macos")]
static CACHED_SPACING: std::sync::OnceLock<f64> = std::sync::OnceLock::new();

#[cfg(target_os = "macos")]
thread_local! {
    // 重入护栏见 align_traffic_lights 文档。
    static ALIGNING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// macOS 在窗口显示/聚焦/缩放时会把红绿灯重置回默认位置,
/// 因此不用 tauri.conf 的 trafficLightPosition,而是自己摆:
/// 把标题栏容器撑到 TITLEBAR_HEIGHT 高,再把三个按钮垂直居中。
///
/// 本函数幂等:已就位时不写任何 frame。这既是性能考虑,
/// 更是防递归——它会被 frame 变更通知回调调用,而写 frame 会同步再发通知。
///
/// 幂等挡不住 AppKit 对打:live resize 期间标题栏布局会把刚写入的按钮
/// 位置同步弹回,每次写入又立刻重入本函数,位置永远"不达标",递归到
/// 栈溢出(实测拖拽窗口边缘缩放时崩溃)。因此再加重入护栏:嵌套通知
/// 一律跳过,弹回后的纠正交给下一个外部事件(Resized/Focused 等)。
///
/// live resize 期间 AppKit 会拒绝 setFrameOrigin,与我们的纠正对打只会
/// 让绿钮在屏幕上乱跳;检测到 inLiveResize 时直接跳过,等松手后的
/// Resized/Focused 再一次性摆回。
#[cfg(target_os = "macos")]
fn align_traffic_lights(ns_window: &objc2_app_kit::NSWindow) {
    if ns_window.inLiveResize() {
        return;
    }
    if ALIGNING.with(|flag| flag.replace(true)) {
        return;
    }
    align_traffic_lights_inner(ns_window);
    ALIGNING.with(|flag| flag.set(false));
}

#[cfg(target_os = "macos")]
fn cache_traffic_light_spacing(ns_window: &objc2_app_kit::NSWindow) {
    use objc2_app_kit::NSWindowButton;
    let _ = CACHED_SPACING.get_or_init(|| unsafe {
        let Some(close) = ns_window.standardWindowButton(NSWindowButton::CloseButton) else {
            return TRAFFIC_LIGHT_SPACING;
        };
        let Some(mini) = ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
            return TRAFFIC_LIGHT_SPACING;
        };
        let spacing = mini.frame().origin.x - close.frame().origin.x;
        if spacing > EPSILON {
            spacing
        } else {
            TRAFFIC_LIGHT_SPACING
        }
    });
}

#[cfg(target_os = "macos")]
fn traffic_light_spacing() -> f64 {
    CACHED_SPACING
        .get()
        .copied()
        .unwrap_or(TRAFFIC_LIGHT_SPACING)
}

#[cfg(target_os = "macos")]
fn align_traffic_lights_inner(ns_window: &objc2_app_kit::NSWindow) {
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

        let spacing = traffic_light_spacing();
        let button_target_y = (TITLEBAR_HEIGHT - close.frame().size.height) / 2.0;
        let mut buttons = vec![close, mini];
        buttons.extend(zoom);
        for (i, button) in buttons.iter().enumerate() {
            let frame = button.frame();
            let target_x = TRAFFIC_LIGHT_X + i as f64 * spacing;
            if (frame.origin.x - target_x).abs() > EPSILON
                || (frame.origin.y - button_target_y).abs() > EPSILON
            {
                button.setFrameOrigin(objc2_foundation::NSPoint::new(target_x, button_target_y));
            }
        }
    }
}

/// 只订阅标题栏容器的 frame 变更;按钮级通知在 live resize 时会与 AppKit 对打。
#[cfg(target_os = "macos")]
pub(crate) fn install(window: &tauri::Window) {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSViewFrameDidChangeNotification, NSWindow, NSWindowButton};
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use std::ptr::NonNull;

    let Ok(ptr) = window.ns_window() else { return };
    unsafe {
        let ns_window = &*(ptr as *const NSWindow);

        cache_traffic_light_spacing(ns_window);

        let observed: Option<Retained<NSView>> = ns_window
            .standardWindowButton(NSWindowButton::CloseButton)
            .and_then(|button| button.superview().and_then(|v| v.superview()));

        let Some(container) = observed else {
            return;
        };

        let center = NSNotificationCenter::defaultCenter();
        container.setPostsFrameChangedNotifications(true);
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
            Some(&container),
            None,
            &block,
        ));

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
