use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, WebviewUrl,
    WebviewWindowBuilder,
};

pub const POPOVER_LABEL: &str = "popover";
// v0.5.0+: popover = 컴팩트 CTA + 최근 활동 프로젝트 4개 퀵셔트 + MCP/종료
// footer. 그 외 패널은 본체에서 보면 됨.
const POPOVER_WIDTH: f64 = 300.0;
// 245: 초기 높이.
// 345: v0.6.0 에 빠른 메모 입력란이 추가되면서 +100.
// 415: v0.7.0 에 Claude Code 사용량 mini 카드가 추가되면서 +70.
// 315: v0.7.0 에 빠른 메모 입력 제거 (popover = 글랜스 + 본체 진입점).
// 사용량 카드는 유지하되 quickInput textarea 슬롯 100px 가 사라져서
// 다시 줄임. 사용량이 0 이면 카드 자체를 안 그려서 바닥이 비어있는
// 형태가 되지만, 한 번 마운트되면 같은 height 라 layout shift 없음.
const POPOVER_HEIGHT: f64 = 315.0;
/// Gap between the top of the screen (menu bar) and the top of the popover.
const MENUBAR_GAP: f64 = 32.0;
/// Horizontal nudge from the tray icon's x position so the popover's right
/// edge doesn't overshoot the screen.
const RIGHT_EDGE_PADDING: f64 = 12.0;

/// Opens the tray popover near the menu bar tray icon, or hides it if already
/// visible. Creates the window on first invocation.
pub fn toggle_popover_window(app: &AppHandle, tray_x: Option<f64>) -> tauri::Result<()> {
    if let Some(win) = app.get_webview_window(POPOVER_LABEL) {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            position_near_tray(&win, tray_x);
            let _ = win.show();
            let _ = win.set_focus();
        }
        return Ok(());
    }

    let url = WebviewUrl::App("index.html#popover".into());
    let win = WebviewWindowBuilder::new(app, POPOVER_LABEL, url)
        .title("단비")
        .inner_size(POPOVER_WIDTH, POPOVER_HEIGHT)
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

    position_near_tray(&win, tray_x);
    let _ = win.show();
    let _ = win.set_focus();

    // Dismiss on blur (Raycast/Dropbox convention).
    let win_for_blur = win.clone();
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            let _ = win_for_blur.hide();
        }
    });
    Ok(())
}

/// Hide without toggling — used by "단비 열기" which opens the main window
/// and should dismiss the popover instead of leaving it lingering.
pub fn hide_popover_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(POPOVER_LABEL) {
        let _ = win.hide();
    }
}

fn position_near_tray(win: &tauri::WebviewWindow, tray_x: Option<f64>) {
    let Some(monitor) = win.current_monitor().ok().flatten() else {
        return;
    };
    let size = monitor.size();
    let scale = monitor.scale_factor();
    let screen_w = size.width as f64 / scale;

    let (monitor_origin_x, monitor_origin_y) = {
        let pos: PhysicalPosition<i32> = *monitor.position();
        (pos.x as f64 / scale, pos.y as f64 / scale)
    };

    // Center below tray icon if we have its x; otherwise right-align.
    let x = match tray_x {
        Some(tx) => {
            // `tx` is a physical-pixel x on the menu bar.
            let logical_tx = tx / scale;
            let centered = logical_tx - POPOVER_WIDTH / 2.0;
            let max_x = monitor_origin_x + screen_w - POPOVER_WIDTH - RIGHT_EDGE_PADDING;
            centered.min(max_x).max(monitor_origin_x + RIGHT_EDGE_PADDING)
        }
        None => monitor_origin_x + screen_w - POPOVER_WIDTH - RIGHT_EDGE_PADDING,
    };
    let y = monitor_origin_y + MENUBAR_GAP;

    let _ = win.set_size(LogicalSize::new(POPOVER_WIDTH, POPOVER_HEIGHT));
    let _ = win.set_position(LogicalPosition::new(x, y));
}
