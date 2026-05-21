use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};

pub const CAPTURE_LABEL: &str = "capture";
const CAPTURE_WIDTH: f64 = 780.0;
const CAPTURE_HEIGHT: f64 = 176.0;
/// Distance in logical pixels between the pill and the bottom of the screen.
/// Must clear the macOS Dock comfortably at default sizes.
const CAPTURE_BOTTOM_MARGIN: f64 = 120.0;

/// Opens or hides the quick-capture popup. Creates it on first invocation.
pub fn toggle_capture_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(win) = app.get_webview_window(CAPTURE_LABEL) {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            position_at_screen_center_bottom(&win);
            let _ = win.show();
            let _ = win.set_focus();
        }
        return Ok(());
    }

    let url = WebviewUrl::App("index.html#capture".into());
    let win = WebviewWindowBuilder::new(app, CAPTURE_LABEL, url)
        .title("단비 Quick Capture")
        .inner_size(CAPTURE_WIDTH, CAPTURE_HEIGHT)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .transparent(true)
        .shadow(false)
        .visible(false)
        .build()?;

    position_at_screen_center_bottom(&win);
    let _ = win.show();
    let _ = win.set_focus();

    // Hide (not close) on blur so the shortcut can quickly restore it.
    let win_for_blur = win.clone();
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            let _ = win_for_blur.hide();
        }
    });
    Ok(())
}

fn position_at_screen_center_bottom(win: &tauri::WebviewWindow) {
    let Some(monitor) = win.current_monitor().ok().flatten() else {
        return;
    };
    let size = monitor.size();
    let scale = monitor.scale_factor();
    let screen_w = size.width as f64 / scale;
    let screen_h = size.height as f64 / scale;
    let x = (screen_w - CAPTURE_WIDTH) / 2.0;
    let y = screen_h - CAPTURE_HEIGHT - CAPTURE_BOTTOM_MARGIN;
    let _ = win.set_size(LogicalSize::new(CAPTURE_WIDTH, CAPTURE_HEIGHT));
    let _ = win.set_position(LogicalPosition::new(x, y));
}

/// 검색 결과 list / 미리보기 등으로 popup 이 커질 때 호출. 항상 화면
/// 하단에서 동일한 마진을 유지하도록 위치도 함께 보정한다 — 그래야
/// 사용자가 입력하는 pill 자체가 같은 자리에 머문다.
const CAPTURE_HEIGHT_MIN: f64 = CAPTURE_HEIGHT;
const CAPTURE_HEIGHT_MAX: f64 = 720.0;

pub fn resize_capture_window(app: &AppHandle, height: f64) -> tauri::Result<()> {
    let Some(win) = app.get_webview_window(CAPTURE_LABEL) else {
        return Ok(());
    };
    let h = height.clamp(CAPTURE_HEIGHT_MIN, CAPTURE_HEIGHT_MAX);
    let Some(monitor) = win.current_monitor().ok().flatten() else {
        let _ = win.set_size(LogicalSize::new(CAPTURE_WIDTH, h));
        return Ok(());
    };
    let size = monitor.size();
    let scale = monitor.scale_factor();
    let screen_w = size.width as f64 / scale;
    let screen_h = size.height as f64 / scale;
    let x = (screen_w - CAPTURE_WIDTH) / 2.0;
    let y = screen_h - h - CAPTURE_BOTTOM_MARGIN;
    let _ = win.set_size(LogicalSize::new(CAPTURE_WIDTH, h));
    let _ = win.set_position(LogicalPosition::new(x, y));
    Ok(())
}
