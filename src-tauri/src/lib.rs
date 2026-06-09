mod aws_profiles;
mod backup;
mod bedrock;
mod capture;
mod commands;
mod compound;
mod config;
mod crash_queue;
mod daily;
mod dashboard;
mod briefing;
mod cache;
mod edit_ops;
mod error;
mod ghost_links;
mod goals;
mod graph;
mod grounding;
mod healing;
mod ingest;
mod journal;
mod journal_view;
mod links;
mod mcp;
mod mcp_inbound;
mod popover;
mod preview;
mod tray_badge;
mod project_context;
mod project_qa;
mod reviews;
mod providers;
mod routing;
mod search;
mod secrets;
mod shortcuts;
mod skill;
mod pricing;
mod templates;
mod trash;
mod exports;
mod usage;
mod vault;
mod vcs;
mod vector;
mod watcher;

use capture::toggle_capture_window;
use popover::{hide_popover_window, toggle_popover_window};

fn default_capture_accelerator() -> std::io::Result<String> {
    let vault = config::default_vault_path()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let loaded = config::load_config(&vault)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    // 사용자가 설정한 값을 그대로. 빈 문자열이면 단축키 비활성 의도이므로
    // 그대로 전달 → 호출부에서 register 스킵. fresh install (config 없음)
    // 도 비활성으로 시작.
    Ok(loaded
        .map(|c| c.shortcuts.quick_capture)
        .unwrap_or_default())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::{Emitter, Manager, WindowEvent};
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        if let Err(e) = toggle_capture_window(app) {
                            eprintln!("capture toggle failed: {e}");
                        }
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();

            // 휴지통 자동 만료 — 30일 넘은 entry 영구 삭제. 앱 시작 시
            // 한 번만 실행. vault path 가 cfg 에 들어있을 때만 동작 —
            // 첫 실행 (아직 vault 없는 상태) 에는 noop. 실패해도 무시.
            if let Ok(default_vault) = config::default_vault_path() {
                if let Ok(Some(cfg)) = config::load_config(&default_vault) {
                    if let Some(vp) = cfg.vault_path.as_ref() {
                        let _ = trash::expire_old(std::path::Path::new(vp));
                    }
                    // Usage retention sweep — trim the live usage log to
                    // the configured retention window. Older lines are
                    // moved to `usage.archive.jsonl` rather than deleted
                    // so the user never loses data they cared about.
                    if cfg.usage.mcp_retention_days > 0 {
                        if let Ok(n) = usage::run_retention_sweep(cfg.usage.mcp_retention_days) {
                            if n > 0 {
                                eprintln!("[usage] retention sweep moved {n} events to archive");
                                mcp_inbound::invalidate_cache();
                            }
                        }
                    }
                }
            }

            // Menubar-only (Accessory) mode on macOS — hides Dock icon and
            // keeps Danbi out of ⌘Tab. Raycast/Alfred convention.
            #[cfg(target_os = "macos")]
            {
                use tauri::ActivationPolicy;
                app.set_activation_policy(ActivationPolicy::Accessory);
            }

            // --- Application menu (macOS top bar) ---
            // Override the default "About Danbi" with a custom event that
            // opens our in-app about dialog. We keep the rest of the macOS
            // defaults (Hide, Quit, Edit/Window submenus) by composing them
            // ourselves — Tauri's `default` menu doesn't expose the About
            // handler cleanly.
            #[cfg(target_os = "macos")]
            {
                let about_item = MenuItem::with_id(
                    &handle,
                    "app:about",
                    "About Danbi",
                    true,
                    None::<&str>,
                )?;
                // macOS-standard "Settings…" (⌘,). Positioned right after
                // About per the Human Interface Guidelines. Fires a
                // `menu:settings` event that the frontend listens for.
                let settings_item = MenuItem::with_id(
                    &handle,
                    "app:settings",
                    "설정…",
                    true,
                    Some("CmdOrCtrl+,"),
                )?;
                let hide = PredefinedMenuItem::hide(&handle, Some("Hide Danbi"))?;
                let hide_others = PredefinedMenuItem::hide_others(&handle, None)?;
                let show_all = PredefinedMenuItem::show_all(&handle, None)?;
                let services = PredefinedMenuItem::services(&handle, None)?;
                let separator = PredefinedMenuItem::separator(&handle)?;
                let separator_after_settings =
                    PredefinedMenuItem::separator(&handle)?;
                let separator2 = PredefinedMenuItem::separator(&handle)?;
                let separator3 = PredefinedMenuItem::separator(&handle)?;
                let separator4 = PredefinedMenuItem::separator(&handle)?;
                let quit = PredefinedMenuItem::quit(&handle, Some("Quit Danbi"))?;
                let app_submenu = Submenu::with_items(
                    &handle,
                    "Danbi",
                    true,
                    &[
                        &about_item,
                        &separator,
                        &settings_item,
                        &separator_after_settings,
                        &services,
                        &separator2,
                        &hide,
                        &hide_others,
                        &show_all,
                        &separator3,
                        &quit,
                        &separator4,
                    ],
                )?;

                // Edit submenu — brings back ⌘C/⌘V/⌘Z etc.
                let undo = PredefinedMenuItem::undo(&handle, None)?;
                let redo = PredefinedMenuItem::redo(&handle, None)?;
                let cut = PredefinedMenuItem::cut(&handle, None)?;
                let copy_i = PredefinedMenuItem::copy(&handle, None)?;
                let paste = PredefinedMenuItem::paste(&handle, None)?;
                let select_all = PredefinedMenuItem::select_all(&handle, None)?;
                let edit_sep = PredefinedMenuItem::separator(&handle)?;
                let edit_submenu = Submenu::with_items(
                    &handle,
                    "Edit",
                    true,
                    &[&undo, &redo, &edit_sep, &cut, &copy_i, &paste, &select_all],
                )?;

                // Window submenu — minimize, fullscreen, bring to front.
                let minimize = PredefinedMenuItem::minimize(&handle, None)?;
                let fullscreen = PredefinedMenuItem::fullscreen(&handle, None)?;
                let window_submenu =
                    Submenu::with_items(&handle, "Window", true, &[&minimize, &fullscreen])?;

                let app_menu = Menu::with_items(
                    &handle,
                    &[&app_submenu, &edit_submenu, &window_submenu],
                )?;
                handle.set_menu(app_menu)?;
                handle.on_menu_event(move |app, event| {
                    match event.id.as_ref() {
                        "app:about" => {
                            if let Some(main) = app.get_webview_window("main") {
                                let _ = main.show();
                                let _ = main.set_focus();
                                let _ = main.emit("about:show", ());
                            }
                        }
                        "app:settings" => {
                            if let Some(main) = app.get_webview_window("main") {
                                let _ = main.show();
                                let _ = main.set_focus();
                                let _ = main.emit("settings:show", ());
                            }
                        }
                        _ => {}
                    }
                });
            }

            // --- Global shortcut registration ---
            // 빈 문자열 = 사용자가 의도적으로 비활성화. fresh install 도
            // 같은 경로 (config 가 없으면 default 가 빈 문자열). 명시적으로
            // 등록한 키만 register 시도.
            let accelerator = default_capture_accelerator().unwrap_or_default();
            if !accelerator.trim().is_empty() {
                if let Err(e) = shortcuts::apply_capture_shortcut(
                    &handle.global_shortcut(),
                    &accelerator,
                ) {
                    eprintln!("register shortcut '{accelerator}' failed: {e}");
                }
            }

            // --- Menu-bar tray ---
            let open_item =
                MenuItem::with_id(&handle, "open", "단비 열기", true, None::<&str>)?;
            let popover_item =
                MenuItem::with_id(&handle, "popover", "팝오버 열기", true, None::<&str>)?;
            let capture_item = MenuItem::with_id(
                &handle,
                "capture",
                "Quick Capture",
                true,
                None::<&str>,
            )?;
            let tray_settings_item = MenuItem::with_id(
                &handle,
                "tray:settings",
                "설정…",
                true,
                None::<&str>,
            )?;
            let sep_settings = PredefinedMenuItem::separator(&handle)?;
            let sep = PredefinedMenuItem::separator(&handle)?;
            let quit_item =
                MenuItem::with_id(&handle, "quit", "종료", true, None::<&str>)?;
            let menu = Menu::with_items(
                &handle,
                &[
                    &open_item,
                    &popover_item,
                    &capture_item,
                    &sep_settings,
                    &tray_settings_item,
                    &sep,
                    &quit_item,
                ],
            )?;

            // Initial tray icon — droplet without a badge. Rendered as
            // a template image so macOS auto-tints for dark/light menu
            // bars. Subsequent count changes swap the icon for one of
            // the pre-rendered badged variants (see tray_badge.rs).
            let initial_icon = tauri::image::Image::from_bytes(include_bytes!(
                "../icons/tray@2x.png"
            ))?;
            let _ = TrayIconBuilder::with_id(tray_badge::TRAY_ID)
                .icon(initial_icon)
                // Not a template image — we want the colored badge to
                // render as-is. The white droplet reads fine on the
                // (always dark) macOS menu bar.
                .icon_as_template(false)
                .tooltip("단비 — Quick Capture")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        hide_popover_window(app);
                        if let Some(main) = app.get_webview_window("main") {
                            let _ = main.show();
                            let _ = main.set_focus();
                        }
                        tray_badge::clear_and_sync(app);
                    }
                    "popover" => {
                        if let Err(e) = toggle_popover_window(app, None) {
                            eprintln!("tray popover toggle failed: {e}");
                        }
                        tray_badge::clear_and_sync(app);
                    }
                    "capture" => {
                        if let Err(e) = toggle_capture_window(app) {
                            eprintln!("tray capture toggle failed: {e}");
                        }
                    }
                    "tray:settings" => {
                        // Surface the main window first, then emit the
                        // same `settings:show` event the macOS menu bar
                        // uses — Workspace listens and opens its
                        // Settings drawer.
                        hide_popover_window(app);
                        if let Some(main) = app.get_webview_window("main") {
                            let _ = main.show();
                            let _ = main.set_focus();
                            let _ = main.emit("settings:show", ());
                        }
                        tray_badge::clear_and_sync(app);
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        // `rect.position` and `rect.size` are `Position` /
                        // `Size` enums holding either Physical or Logical
                        // variants. Convert both to physical pixels — the
                        // popover window positioner expects physical x.
                        let scale = tray
                            .app_handle()
                            .primary_monitor()
                            .ok()
                            .flatten()
                            .map(|m| m.scale_factor())
                            .unwrap_or(1.0);
                        let pos = rect.position.to_physical::<f64>(scale);
                        let sz = rect.size.to_physical::<f64>(scale);
                        let center_x = pos.x + sz.width / 2.0;
                        if let Err(e) =
                            toggle_popover_window(tray.app_handle(), Some(center_x))
                        {
                            eprintln!("tray popover toggle failed: {e}");
                        }
                        tray_badge::clear_and_sync(tray.app_handle());
                    }
                })
                .build(app)?;

            // --- MCP server (opt-in) ---
            if let Ok(default_vault) = config::default_vault_path() {
                if let Ok(Some(cfg)) = config::load_config(&default_vault) {
                    if cfg.mcp.enabled {
                        let token = mcp::ensure_token(&cfg);
                        // If token was just generated, persist it.
                        if token != cfg.mcp.token {
                            let mut next = cfg.clone();
                            next.mcp.token = token.clone();
                            if let Some(path) = next.vault_path.as_ref() {
                                let _ = config::save_config(
                                    &std::path::PathBuf::from(path),
                                    &next,
                                );
                            }
                        }
                        let server = handle.state::<mcp::McpServer>();
                        server.start(cfg.mcp.port, token);
                    }
                }
            }

            // --- Keep running in the background when the main window closes ---
            if let Some(main) = app.get_webview_window("main") {
                let main_handle = handle.clone();
                main.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        // Hide instead of quitting; the tray + global shortcut
                        // keep the app alive.
                        api.prevent_close();
                        if let Some(win) = main_handle.get_webview_window("main") {
                            let _ = win.hide();
                        }
                    }
                });
            }

            // Reset any crash-queue records that were marked Running when
            // the previous process exited. Converts them to Pending so the
            // next UI interaction (or the user directly) can resume them.
            if let Ok(default_vault) = config::default_vault_path() {
                if let Ok(Some(cfg)) = config::load_config(&default_vault) {
                    if let Some(vp) = cfg.vault_path.as_ref() {
                        let _ = crash_queue::reset_stale(
                            &std::path::PathBuf::from(vp),
                        );
                    }
                }
            }

            Ok(())
        })
        .manage(watcher::WatcherState::new())
        .manage(mcp::McpServer::new())
        .manage(tray_badge::TrayBadgeState::new(
            config::default_vault_path()
                .ok()
                .and_then(|v| config::load_config(&v).ok().flatten())
                .map(|c| c.appearance.tray_badge)
                .unwrap_or(true),
        ))
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::default_vault,
            commands::load_config,
            commands::save_config,
            commands::detect_aws,
            commands::store_manual_credentials,
            commands::delete_manual_credentials,
            commands::list_bedrock_models,
            commands::test_bedrock,
            commands::store_nvidia_api_key,
            commands::delete_nvidia_api_key,
            commands::list_nvidia_models,
            commands::test_nvidia,
            commands::store_openai_api_key,
            commands::delete_openai_api_key,
            commands::list_openai_models,
            commands::test_openai,
            commands::store_anthropic_api_key,
            commands::delete_anthropic_api_key,
            commands::list_anthropic_models,
            commands::test_anthropic,
            commands::store_google_api_key,
            commands::delete_google_api_key,
            commands::list_google_models,
            commands::test_google,
            commands::list_ollama_models,
            commands::test_ollama,
            commands::store_voyage_api_key,
            commands::delete_voyage_api_key,
            commands::list_voyage_models,
            commands::test_voyage,
            commands::dashboard_snapshot,
            commands::dashboard_mcp_inbound,
            commands::dashboard_mcp_inbound_project,
            commands::dashboard_mcp_inbound_domain,
            commands::usage_export_json,
            commands::usage_export_csv,
            commands::usage_retention_sweep,
            commands::usage_set_mcp_tracking,
            commands::usage_set_mcp_retention,
            commands::build_graph,
            commands::backup_now,
            commands::backup_validate_path,
            commands::project_context_status,
            commands::project_context_ensure,
            commands::cache_clear,
            commands::reviews_list,
            commands::reviews_resolve,
            commands::vector_stats,
            commands::vector_reindex,
            commands::vector_reindex_project,
            commands::vector_estimate_reindex,
            commands::usage_month_to_date,
            commands::usage_set_rate,
            commands::project_journal_view,
            commands::project_updates,
            commands::project_mark_seen,
            commands::domain_updates,
            commands::domain_mark_seen,
            commands::project_mark_all_read,
            commands::vault_mark_all_read,
            commands::groups_set,
            commands::vector_clear,
            commands::vector_search,
            commands::init_vault,
            commands::list_tree,
            commands::create_project,
            commands::rename_project,
            commands::delete_project,
            commands::create_domain,
            commands::rename_domain,
            commands::delete_domain,
            commands::create_folder,
            commands::rename_folder,
            commands::delete_folder,
            commands::move_domain,
            commands::install_skill,
            commands::skill_status,
            commands::trash_list,
            commands::trash_restore,
            commands::trash_purge,
            commands::trash_empty,
            commands::read_doc,
            commands::write_doc,
            commands::save_asset,
            commands::resolve_asset,
            commands::start_watching,
            commands::stop_watching,
            commands::route_message,
            commands::preview_plan,
            commands::apply_plan,
            commands::undo_last,
            commands::recent_commits,
            commands::build_link_index,
            commands::vault_suggestions,
            commands::daily_snapshot,
            commands::ensure_today_note,
            commands::list_templates,
            commands::apply_template,
            commands::mcp_status,
            commands::mcp_enable,
            commands::mcp_disable,
            commands::mcp_rotate_token,
            commands::mcp_project_endpoint,
            commands::quick_capture,
            commands::toggle_capture,
            commands::hide_capture,
            commands::resize_capture,
            commands::capture_open_hit,
            commands::summarize_daily,
            commands::export_doc_html,
            commands::open_html_preview,
            commands::save_html_to_path,
            commands::list_exports,
            commands::open_export,
            commands::compose_purpose_schema,
            commands::hide_popover,
            commands::open_main_window,
            commands::autostart_status,
            commands::autostart_set,
            commands::tray_badge_count,
            commands::tray_badge_reset,
            commands::tray_badge_set_enabled,
            commands::apply_capture_shortcut,
            commands::validate_shortcut,
            commands::capture_context,
            commands::search_vault,
            commands::search_local,
            commands::search_full,
            commands::compound_preview,
            commands::compound_apply,
            commands::ghost_list,
            commands::ghost_scan,
            commands::ghost_accept,
            commands::ghost_reject,
            commands::project_qa_ask,
            commands::project_briefing,
            commands::extract_file_path,
            commands::extract_file_bytes,
            commands::goals_list,
            commands::goals_add,
            commands::goals_edit,
            commands::goals_archive,
            commands::goals_unarchive,
            commands::goals_delete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
