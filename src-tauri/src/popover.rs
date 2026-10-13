use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, WebviewUrl, WebviewWindowBuilder,
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
// 365: v0.8.7 에 사용량 카드 아래 계정별(~/.claude-<계정>) 2열 목록이
// 붙으면서 +50 (계정 6개 = 3줄 기준).
const POPOVER_HEIGHT: f64 = 365.0;
/// Gap between the top of the screen (menu bar) and the top of the popover.
const MENUBAR_GAP: f64 = 32.0;
/// Minimum gap between the popover and the left/right edge of the screen.
const RIGHT_EDGE_PADDING: f64 = 12.0;

/// Opens the tray popover under the mouse cursor on the menu bar, or hides it
/// if already visible. Creates the window on first invocation.
pub fn toggle_popover_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(win) = app.get_webview_window(POPOVER_LABEL) {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            position_near_cursor(&win);
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

    position_near_cursor(&win);
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

/// Logical (point) bounds of a monitor: `(x, y, width, height)`.
///
/// tao reports each monitor's position/size as logical points multiplied by
/// *that monitor's own* scale factor, so dividing by the same factor gives
/// back the global point-space rectangle macOS uses for window placement.
fn logical_bounds(m: &Monitor) -> (f64, f64, f64, f64) {
    let s = m.scale_factor();
    let pos = m.position();
    let size = m.size();
    (
        pos.x as f64 / s,
        pos.y as f64 / s,
        size.width as f64 / s,
        size.height as f64 / s,
    )
}

/// Finds the monitor under the mouse cursor and returns it with the cursor x
/// in logical points.
///
/// tao's `cursor_position()` is `NSEvent mouseLocation` (global points)
/// scaled by the *primary* monitor's factor, so dividing by that factor
/// gives exact points regardless of each display's own scale. (The tray
/// event's `position` is scaled by the clicked menu bar's factor instead,
/// which is ambiguous with mixed-scale displays.)
fn locate_cursor(win: &tauri::WebviewWindow) -> Option<(Monitor, f64)> {
    let cursor = win.cursor_position().ok()?;
    let s = win.primary_monitor().ok().flatten()?.scale_factor();
    let (x, y) = (cursor.x / s, cursor.y / s);
    win.available_monitors().ok()?.into_iter().find_map(|m| {
        let (mx, my, mw, mh) = logical_bounds(&m);
        (x >= mx && x < mx + mw && y >= my && y < my + mh).then_some((m, x))
    })
}

/// Places the popover under the cursor, on the monitor that was clicked.
///
/// macOS 27 + multiple displays: the tray icon rect from the click event can
/// come back in another display's coordinates (the icon x flipped between
/// 2053 and 4613 on a 3-monitor setup), which pushed the popover against the
/// right edge of the wrong screen. The cursor (`NSEvent mouseLocation`) is
/// global and reliable, so we anchor to it instead.
fn position_near_cursor(win: &tauri::WebviewWindow) {
    let (monitor, cursor_x) = match locate_cursor(win) {
        Some((m, x)) => (m, Some(x)),
        None => match win.current_monitor().ok().flatten() {
            Some(m) => (m, None),
            None => return,
        },
    };
    let (mx, my, mw, _) = logical_bounds(&monitor);

    // Center below the click if we have it; otherwise right-align.
    let max_x = mx + mw - POPOVER_WIDTH - RIGHT_EDGE_PADDING;
    let x = match cursor_x {
        Some(cx) => (cx - POPOVER_WIDTH / 2.0)
            .min(max_x)
            .max(mx + RIGHT_EDGE_PADDING),
        None => max_x,
    };
    let y = my + MENUBAR_GAP;

    let _ = win.set_size(LogicalSize::new(POPOVER_WIDTH, POPOVER_HEIGHT));
    let _ = win.set_position(LogicalPosition::new(x, y));
}
