mod watcher;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use cc_monitor::{api, blocks::SessionBlock, model::SessionRow};
use serde::{Deserialize, Serialize};
use tauri::{
    ActivationPolicy, AppHandle, Manager, PhysicalPosition, WindowEvent,
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

fn active_block_cost() -> Option<f64> {
    api::list_blocks(None)
        .ok()?
        .into_iter()
        .find(|b| b.is_active)
        .and_then(|b| {
            let model = b.models.first().cloned()?;
            let table = cc_monitor::pricing::load();
            let p = cc_monitor::pricing::lookup(&table, &model)?;
            Some(cc_monitor::pricing::cost_usd(p, &b.tokens))
        })
}

fn tray_title() -> String {
    let cost = active_block_cost().unwrap_or(0.0);
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
    format!("${cost:.2} · {live} live")
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

fn check_budget(app: &AppHandle) {
    let settings = load_settings(app);
    if settings.budget_window_usd <= 0.0 {
        eprintln!("[budget] disabled (threshold=0)");
        return;
    }
    let Some(cost) = active_block_cost() else {
        eprintln!("[budget] no active block");
        return;
    };
    eprintln!(
        "[budget] cost=${cost:.2} threshold=${:.2}",
        settings.budget_window_usd
    );
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

            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("missing default window icon")?;

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
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            list_blocks,
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
