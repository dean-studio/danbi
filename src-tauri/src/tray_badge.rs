use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tauri::image::Image;
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Manager};

pub const TRAY_ID: &str = "danbi-tray";

// Pre-rendered tray PNGs. Compiled in so the tray icon swap is just a
// memory blob — no disk read at swap time. Each pair is 1x (22) + 2x
// (44). macOS picks the right one for the active display automatically
// when both are wrapped in the same NSImage; for Tauri we hand it the
// 2x version since menu-bar height on retina is the common case.
//
// 9+ caps the badge so we don't try to render arbitrary digit strings
// that would overflow the small lower-left disc. Any count >= 10 uses
// the same 9+ asset.
const TRAY_BASE_2X: &[u8] = include_bytes!("../icons/tray@2x.png");
const TRAY_1_2X: &[u8] = include_bytes!("../icons/tray-1@2x.png");
const TRAY_2_2X: &[u8] = include_bytes!("../icons/tray-2@2x.png");
const TRAY_3_2X: &[u8] = include_bytes!("../icons/tray-3@2x.png");
const TRAY_4_2X: &[u8] = include_bytes!("../icons/tray-4@2x.png");
const TRAY_5_2X: &[u8] = include_bytes!("../icons/tray-5@2x.png");
const TRAY_6_2X: &[u8] = include_bytes!("../icons/tray-6@2x.png");
const TRAY_7_2X: &[u8] = include_bytes!("../icons/tray-7@2x.png");
const TRAY_8_2X: &[u8] = include_bytes!("../icons/tray-8@2x.png");
const TRAY_9_2X: &[u8] = include_bytes!("../icons/tray-9@2x.png");
const TRAY_9PLUS_2X: &[u8] = include_bytes!("../icons/tray-9plus@2x.png");

fn icon_bytes(n: usize) -> &'static [u8] {
    match n {
        0 => TRAY_BASE_2X,
        1 => TRAY_1_2X,
        2 => TRAY_2_2X,
        3 => TRAY_3_2X,
        4 => TRAY_4_2X,
        5 => TRAY_5_2X,
        6 => TRAY_6_2X,
        7 => TRAY_7_2X,
        8 => TRAY_8_2X,
        9 => TRAY_9_2X,
        _ => TRAY_9PLUS_2X,
    }
}

/// Global state for the tray badge. The counter is separate from the
/// enable flag so toggling the setting off doesn't lose the tally (we
/// just stop rendering it).
#[derive(Default)]
pub struct TrayBadgeState {
    /// Number of markdown file changes observed since the last reset.
    count: AtomicUsize,
    /// Whether the badge should be rendered on the tray icon.
    enabled: Mutex<bool>,
}

impl TrayBadgeState {
    pub fn new(enabled: bool) -> Self {
        Self {
            count: AtomicUsize::new(0),
            enabled: Mutex::new(enabled),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.lock().map(|g| *g).unwrap_or(true)
    }

    pub fn set_enabled(&self, v: bool) {
        if let Ok(mut g) = self.enabled.lock() {
            *g = v;
        }
    }

    pub fn count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    pub fn bump(&self, by: usize) {
        self.count.fetch_add(by, Ordering::Relaxed);
    }

    pub fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
    }
}

/// Apply the current counter to the tray icon. Picks the pre-rendered
/// PNG matching the count (or the unbadged base if `enabled` is off).
/// Safe to call frequently — Tauri's set_icon is a cheap pointer swap.
pub fn sync_tray_title(app: &AppHandle) {
    let state = app.state::<TrayBadgeState>();
    let n = if state.is_enabled() { state.count() } else { 0 };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        apply_icon(&tray, n);
    }
}

fn apply_icon(tray: &TrayIcon, n: usize) {
    if let Ok(img) = Image::from_bytes(icon_bytes(n)) {
        let _ = tray.set_icon(Some(img));
    }
    // Make sure the title slot stays empty — we previously used it for
    // the textual "단 N" badge. Clearing here is idempotent.
    let _ = tray.set_title(None::<&str>);
}

/// Called whenever the user surfaces Danbi (popover or main window). We
/// zero the counter and repaint the tray.
pub fn clear_and_sync(app: &AppHandle) {
    let state = app.state::<TrayBadgeState>();
    state.reset();
    sync_tray_title(app);
}
