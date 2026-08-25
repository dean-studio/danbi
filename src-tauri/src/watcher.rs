use crate::config;
use crate::error::{DanbiError, DanbiResult};
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub const VAULT_CHANGED_EVENT: &str = "vault:changed";

/// True for paths we want to show up in the tray badge counter. We only
/// care about user-facing `.md` files inside the vault — not git
/// internals, not `.danbi/` metadata, not `_assets/` binary churn.
fn is_tracked_markdown(vault: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(vault) else {
        return false;
    };
    if rel.as_os_str().is_empty() {
        return false;
    }
    for seg in rel.iter() {
        let Some(s) = seg.to_str() else {
            return false;
        };
        // Hidden folders (.git / .danbi) and the asset sideload directory
        // should not bump the badge.
        if s.starts_with('.') || s == "_assets" {
            return false;
        }
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("md")
    )
}

/// Should this filesystem change cause the frontend to refresh its tree
/// + link index? We answer "no" for noisy paths the user never sees:
///  - `.git/`, `.danbi/` internals
///  - `_assets/` (binary churn)
///  - `config.json` / `history.jsonl` / `log.md` at the vault root —
///    these mutate on every click (project_mark_seen, etc.) and cause a
///    cascade of refreshes that visibly stalls the sidebar.
///
/// Tree-relevant changes are: new/renamed/deleted folders, new/renamed/
/// deleted/edited `.md` files, and the rare `.gitkeep` placeholder.
fn is_tree_relevant(vault: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(vault) else {
        return false;
    };
    if rel.as_os_str().is_empty() {
        return false;
    }
    for seg in rel.iter() {
        let Some(s) = seg.to_str() else {
            return false;
        };
        if s.starts_with('.') || s == "_assets" {
            return false;
        }
    }
    // Vault root metadata files — config saves itself on every click.
    if let Some(top) = rel.iter().next().and_then(|s| s.to_str()) {
        if rel.iter().count() == 1
            && (top == "config.json" || top == "history.jsonl" || top == "log.md")
        {
            return false;
        }
    }
    // Either a directory event (rename/create/delete shows up as a path
    // with no extension) or a markdown file edit. Anything else (binary
    // attachments, lockfiles) we ignore.
    let is_md = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("md")
    );
    is_md || path.extension().is_none()
}

/// Shared "next scheduled backup" timestamp. Every vault-change event
/// pushes this forward by `debounce_ms`; a tiny worker thread polls it
/// and runs the backup when the deadline has passed with no further
/// movement. This gives us rsync-style collapse of bursty edits.
#[derive(Clone)]
struct BackupScheduler {
    inner: Arc<Mutex<Option<Instant>>>,
}

impl BackupScheduler {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }
    fn schedule(&self, debounce: Duration) {
        if let Ok(mut slot) = self.inner.lock() {
            *slot = Some(Instant::now() + debounce);
        }
    }
    /// Returns `Some` only when the deadline has arrived and we should
    /// actually fire. Consumes the slot so overlapping wakeups don't
    /// trigger duplicate runs.
    fn take_due(&self) -> bool {
        if let Ok(mut slot) = self.inner.lock() {
            if let Some(deadline) = *slot {
                if Instant::now() >= deadline {
                    *slot = None;
                    return true;
                }
            }
        }
        false
    }
}

pub struct WatcherState {
    inner: Mutex<Option<WatcherHandle>>,
}

impl WatcherState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
}

struct WatcherHandle {
    path: PathBuf,
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    /// Shutdown flag for the backup worker thread. Set on drop so a
    /// stop/restart doesn't orphan the previous worker.
    stop: Arc<AtomicBool>,
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        // Signal the backup worker to exit its loop. It polls this once
        // per second, so it winds down within ~1s of the watcher stopping.
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn build_debouncer(
    path: &Path,
    app: AppHandle,
    scheduler: BackupScheduler,
) -> DanbiResult<Debouncer<notify::RecommendedWatcher, RecommendedCache>> {
    let watch_path = path.to_path_buf();
    let sched_for_events = scheduler.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(250),
        None,
        move |res: DebounceEventResult| {
            if let Ok(events) = res {
                // Collapse events to a single signal with the vault path.
                // Front-end responds by re-fetching list_tree and the open doc.
                if !events.is_empty() {
                    // Only emit when at least one path is *tree-relevant*.
                    // This prevents the sidebar from re-rendering on every
                    // click (project_mark_seen rewrites config.json, which
                    // would otherwise fire vault:changed and trigger a full
                    // tree+linkIndex+groups refresh — the source of the
                    // perceived "lag" when expanding folders or selecting
                    // files in big vaults).
                    let tree_relevant = events.iter().any(|ev| {
                        ev.event
                            .paths
                            .iter()
                            .any(|p| is_tree_relevant(&watch_path, p))
                    });
                    if tree_relevant {
                        let _ = app.emit(
                            VAULT_CHANGED_EVENT,
                            serde_json::json!({
                                "vault": watch_path.to_string_lossy().to_string(),
                                "count": events.len(),
                            }),
                        );
                    }

                    // Count *distinct markdown paths* that showed up in
                    // this debounce batch. Each path counts once even if
                    // it fired multiple notify events. Paths inside
                    // `.git`, `.danbi`, or `_assets` are ignored — those
                    // are internal churn the user doesn't care about.
                    let mut md_paths: std::collections::HashSet<PathBuf> =
                        std::collections::HashSet::new();
                    for ev in &events {
                        for p in &ev.event.paths {
                            if is_tracked_markdown(&watch_path, p) {
                                md_paths.insert(p.clone());
                            }
                        }
                    }
                    if !md_paths.is_empty() {
                        // Skip bumps while the main window is focused —
                        // the user is clearly active, the badge would
                        // just be noise.
                        let main_focused = app
                            .get_webview_window("main")
                            .and_then(|w| w.is_focused().ok())
                            .unwrap_or(false);
                        if !main_focused {
                            let state =
                                app.state::<crate::tray_badge::TrayBadgeState>();
                            state.bump(md_paths.len());
                            crate::tray_badge::sync_tray_title(&app);
                        }
                    }

                    // Also push the backup deadline forward so the mirror
                    // stays eventually consistent with the vault. The
                    // worker thread (spawned in `start`) reads this.
                    if let Ok(Some(cfg)) = config::load_config(&watch_path) {
                        if cfg.backup.enabled && cfg.backup.path.is_some() {
                            sched_for_events
                                .schedule(Duration::from_millis(cfg.backup.debounce_ms));
                        }
                    }
                }
            }
        },
    )
    .map_err(|e| DanbiError::Other(format!("watcher: {e}")))?;

    debouncer
        .watch(path, RecursiveMode::Recursive)
        .map_err(|e| DanbiError::Other(format!("watch: {e}")))?;
    Ok(debouncer)
}

/// Runs forever in a dedicated thread. Polls the scheduler every second
/// and, when due, re-loads config and fires a backup pass. We re-load
/// config on every run so toggling the enabled flag in Settings takes
/// effect on the very next tick without restarting the watcher.
fn spawn_backup_worker(
    vault_path: PathBuf,
    scheduler: BackupScheduler,
    stop: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            // Exit cleanly once the owning WatcherHandle is dropped —
            // prevents thread accumulation across stop/restart cycles.
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if !scheduler.take_due() {
                continue;
            }
            let Ok(Some(mut cfg)) = config::load_config(&vault_path) else {
                continue;
            };
            if !cfg.backup.enabled {
                continue;
            }
            let Some(dest) = cfg.backup.path.clone() else {
                continue;
            };
            match crate::backup::run(
                &vault_path,
                &PathBuf::from(&dest),
                &cfg.backup.exclude,
            ) {
                Ok(report) => {
                    cfg.backup.last_run_at = Some(chrono::Local::now().timestamp());
                    cfg.backup.last_message = Some(format!(
                        "{} copied · {} skipped · {} removed",
                        report.copied, report.skipped, report.removed
                    ));
                }
                Err(e) => {
                    cfg.backup.last_run_at = Some(chrono::Local::now().timestamp());
                    cfg.backup.last_message = Some(format!("실패: {e}"));
                }
            }
            let _ = config::save_config(&vault_path, &cfg);
        }
    });
}

/// Start (or restart) watching the given vault directory. Only one watcher runs at a time.
pub fn start(app: &AppHandle, path: &Path) -> DanbiResult<()> {
    let state = app.state::<WatcherState>();
    let mut slot = state.inner.lock().map_err(|_| DanbiError::Other("watcher lock".into()))?;

    if let Some(existing) = slot.as_ref() {
        if existing.path == path {
            return Ok(());
        }
    }
    // drop old watcher before starting a new one
    slot.take();

    let scheduler = BackupScheduler::new();
    let debouncer = build_debouncer(path, app.clone(), scheduler.clone())?;
    let stop = Arc::new(AtomicBool::new(false));
    spawn_backup_worker(path.to_path_buf(), scheduler, stop.clone());
    *slot = Some(WatcherHandle {
        path: path.to_path_buf(),
        _debouncer: debouncer,
        stop,
    });
    Ok(())
}

pub fn stop(app: &AppHandle) -> DanbiResult<()> {
    let state = app.state::<WatcherState>();
    let mut slot = state.inner.lock().map_err(|_| DanbiError::Other("watcher lock".into()))?;
    slot.take();
    Ok(())
}
