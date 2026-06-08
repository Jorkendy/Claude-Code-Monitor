mod watcher;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use cc_monitor::{api, blocks::SessionBlock, model::SessionRow};
use serde::{Deserialize, Serialize};
use tauri::{
    ActivationPolicy, AppHandle, Manager, PhysicalPosition, WindowEvent,
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_notification::{NotificationExt, PermissionState};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Settings {
    /// Notify when current 5h block cost exceeds this threshold (USD).
    /// 0 disables the alert.
    budget_window_usd: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            budget_window_usd: 5.0,
        }
    }
}

fn settings_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("settings.json"))
}

fn load_settings(app: &AppHandle) -> Settings {
    let Some(path) = settings_path(app) else {
        return Settings::default();
    };
    std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn list_sessions() -> Result<Vec<SessionRow>, String> {
    api::list_sessions(None).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_blocks() -> Result<Vec<SessionBlock>, String> {
    api::list_blocks(None).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
struct BlockView {
    #[serde(flatten)]
    block: SessionBlock,
    /// Cost computed from first non-empty model in the block. Matches
    /// the CLI's `block_cost` heuristic in renderer.rs.
    cost_usd: f64,
    /// Rolling burn rate in USD/hr (last 10 min). Set for the active block
    /// only; 0 for done blocks.
    burn_usd_per_hr: f64,
    /// `current cost + burn × hours-until-block-reset`. Active block only.
    projected_block_usd: f64,
}

#[tauri::command]
fn list_block_views() -> Result<Vec<BlockView>, String> {
    let enriched = api::list_blocks_enriched(None).map_err(|e| e.to_string())?;
    let table = cc_monitor::pricing::load();
    Ok(enriched
        .blocks
        .into_iter()
        .map(|b| {
            let cost = b
                .models
                .iter()
                .find(|m| !m.is_empty())
                .and_then(|m| cc_monitor::pricing::lookup(&table, m))
                .map(|p| cc_monitor::pricing::cost_usd(p, &b.tokens))
                .unwrap_or(0.0);
            let (burn, projected) = if b.is_active {
                enriched
                    .active
                    .as_ref()
                    .map(|a| (a.burn_usd_per_hr, a.projected_block_usd))
                    .unwrap_or((0.0, cost))
            } else {
                (0.0, 0.0)
            };
            BlockView {
                block: b,
                cost_usd: cost,
                burn_usd_per_hr: burn,
                projected_block_usd: projected,
            }
        })
        .collect())
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Settings {
    load_settings(&app)
}

#[tauri::command]
fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let path = settings_path(&app).ok_or("no config dir")?;
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(())
}

fn active_burn() -> Option<api::ActiveBurn> {
    api::list_blocks_enriched(None).ok()?.active
}

fn tray_title() -> String {
    let burn = active_burn();
    let cost = burn.as_ref().map(|a| a.current_usd).unwrap_or(0.0);
    let rate = burn.as_ref().map(|a| a.burn_usd_per_hr).unwrap_or(0.0);
    let live = api::list_sessions(None)
        .map(|rows| {
            rows.iter()
                .filter(|r| {
                    matches!(
                        r.status,
                        cc_monitor::model::LiveStatus::Active | cc_monitor::model::LiveStatus::Idle
                    )
                })
                .count()
        })
        .unwrap_or(0);
    // Only show $/hr when meaningful — sub-$0.50/hr is just noise.
    if rate >= 0.5 {
        format!("${cost:.2} · ${rate:.0}/hr · {live} live")
    } else {
        format!("${cost:.2} · {live} live")
    }
}

fn refresh_tray(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_title(Some(tray_title()));
    }
}

// One alert per crossing: after firing, suppress until the cost dips back
// below the threshold (or 1h passes, as a safety reset for long sessions).
static LAST_ALERT: Mutex<Option<Instant>> = Mutex::new(None);
static ALERT_FIRED_FOR_CURRENT_CROSSING: Mutex<bool> = Mutex::new(false);
// Proactive projection alert: at most one per block. Keyed by block.start_ms
// so we re-arm when the 5h window rolls over.
static PROJ_ALERTED_BLOCK_START: Mutex<Option<i64>> = Mutex::new(None);

fn check_budget(app: &AppHandle) {
    let settings = load_settings(app);
    if settings.budget_window_usd <= 0.0 {
        eprintln!("[budget] disabled (threshold=0)");
        return;
    }
    let Some(burn) = active_burn() else {
        eprintln!("[budget] no active block");
        return;
    };
    let cost = burn.current_usd;
    let projected = burn.projected_block_usd;
    eprintln!(
        "[budget] cost=${cost:.2} proj=${projected:.2} threshold=${:.2}",
        settings.budget_window_usd
    );
    // Re-arm projection alert when block boundary changes.
    {
        let mut last_block = PROJ_ALERTED_BLOCK_START.lock().unwrap();
        if *last_block != Some(burn.block_start_ms) {
            *last_block = None;
        }
    }
    // Proactive: projected to cross but actual hasn't yet → tell user once
    // per block, so they can pause before they're surprised.
    if projected >= settings.budget_window_usd && cost < settings.budget_window_usd {
        let mut last_block = PROJ_ALERTED_BLOCK_START.lock().unwrap();
        if *last_block != Some(burn.block_start_ms) {
            let res = app
                .notification()
                .builder()
                .title("cc-monitor — projected over budget")
                .body(format!(
                    "Block trending to ${projected:.2} by reset (threshold ${:.2}, burn ${:.0}/hr)",
                    settings.budget_window_usd, burn.burn_usd_per_hr
                ))
                .show();
            eprintln!("[budget] projection notification => {res:?}");
            *last_block = Some(burn.block_start_ms);
        }
    }
    let mut fired = ALERT_FIRED_FOR_CURRENT_CROSSING.lock().unwrap();
    let mut last = LAST_ALERT.lock().unwrap();
    if cost < settings.budget_window_usd {
        *fired = false;
        return;
    }
    let stale = last.map(|t| t.elapsed() > Duration::from_secs(3600)).unwrap_or(true);
    if !*fired || stale {
        let res = app
            .notification()
            .builder()
            .title("cc-monitor")
            .body(format!(
                "Current 5h block at ${cost:.2} (threshold ${:.2})",
                settings.budget_window_usd
            ))
            .show();
        eprintln!("[budget] notification show => {res:?}");
        *fired = true;
        *last = Some(Instant::now());
    }
}

fn ensure_notification_permission(app: &AppHandle) {
    match app.notification().permission_state() {
        Ok(PermissionState::Granted) => {
            eprintln!("[notify] permission already granted");
        }
        Ok(other) => {
            eprintln!("[notify] permission state = {other:?}, requesting…");
            match app.notification().request_permission() {
                Ok(state) => eprintln!("[notify] permission after request = {state:?}"),
                Err(e) => eprintln!("[notify] request_permission error: {e}"),
            }
        }
        Err(e) => eprintln!("[notify] permission_state error: {e}"),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            let _ = app.set_activation_policy(ActivationPolicy::Accessory);

            let quit = MenuItem::with_id(app, "quit", "Quit cc-monitor", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            // Custom monochrome "C" icon for the menubar, rendered as a
            // macOS template image so the system tints it for light/dark
            // menubar. Bundled into the binary so no runtime path lookup.
            let icon = Image::from_bytes(include_bytes!("../icons/tray/tray@2x.png"))?;

            TrayIconBuilder::with_id("main")
                .icon(icon)
                .icon_as_template(true)
                .title(tray_title())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id().as_ref() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        toggle_window(tray, rect);
                    }
                })
                .build(app)?;

            // Force registration with the macOS Notification Center.
            ensure_notification_permission(&app.handle());

            // Spawn the file-watcher; debounced refreshes update the tray,
            // emit a frontend event, and check the budget threshold.
            let root = api::resolve_root(None)?;
            eprintln!("[watcher] watching root={}", root.display());
            watcher::spawn(app.handle().clone(), root, |app| {
                eprintln!("[watcher] data-changed fired");
                refresh_tray(app);
                check_budget(app);
            });

            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                let _ = window.hide();
                api.prevent_close();
            }
            // Menubar UX: clicking anywhere outside the popover dismisses it.
            // 200ms debounce so transient focus losses (notifications, tray
            // re-toggle) don't accidentally dismiss it — if the window regains
            // focus before the timer fires, we skip the hide.
            WindowEvent::Focused(false) => {
                let window = window.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(200));
                    if !window.is_focused().unwrap_or(true) {
                        let _ = window.hide();
                    }
                });
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            list_blocks,
            list_block_views,
            get_settings,
            set_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn toggle_window(tray: &tauri::tray::TrayIcon, rect: tauri::Rect) {
    let app = tray.app_handle();
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let icon_pos = rect.position.to_physical::<i32>(scale);
    let icon_size = rect.size.to_physical::<u32>(scale);
    let icon_center_x = icon_pos.x + (icon_size.width as i32) / 2;
    let icon_bottom_y = icon_pos.y + icon_size.height as i32;
    let win_w = window
        .outer_size()
        .map(|s| s.width as i32)
        .unwrap_or(460);
    let x = icon_center_x - (win_w / 2);
    let y = icon_bottom_y + 6;
    let _ = window.set_position(PhysicalPosition::new(x, y));
    let _ = window.show();
    let _ = window.set_focus();
}
