mod watcher;

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokenscope::{api, blocks::SessionBlock, model::SessionRow};
use serde::{Deserialize, Serialize};
use tauri::{
    ActivationPolicy, AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
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
    let table = tokenscope::pricing::load();
    Ok(enriched
        .blocks
        .into_iter()
        .map(|b| {
            let cost = b
                .models
                .iter()
                .find(|m| !m.is_empty())
                .and_then(|m| tokenscope::pricing::lookup(&table, m))
                .map(|p| tokenscope::pricing::cost_usd(p, &b.tokens))
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

// ---- Hidden sessions ----------------------------------------------------
//
// User-driven soft-delete: the file in ~/.claude/ stays put, we just stop
// showing the session_id in either window. Persisted at
// `app_config_dir/hidden_sessions.json`.

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct HiddenFile {
    #[serde(default)]
    ids: Vec<String>,
}

fn hidden_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("hidden_sessions.json"))
}

fn load_hidden(app: &AppHandle) -> HiddenFile {
    let Some(path) = hidden_path(app) else {
        return HiddenFile::default();
    };
    std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_hidden(app: &AppHandle, h: &HiddenFile) -> Result<(), String> {
    let path = hidden_path(app).ok_or("no config dir")?;
    let bytes = serde_json::to_vec_pretty(h).map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_hidden(app: AppHandle) -> Vec<String> {
    load_hidden(&app).ids
}

#[tauri::command]
fn hide_session(app: AppHandle, session_id: String) -> Result<(), String> {
    let mut h = load_hidden(&app);
    if !h.ids.contains(&session_id) {
        h.ids.push(session_id);
        save_hidden(&app, &h)?;
        let _ = app.emit("data-changed", ());
    }
    Ok(())
}

#[tauri::command]
fn unhide_session(app: AppHandle, session_id: String) -> Result<(), String> {
    let mut h = load_hidden(&app);
    let before = h.ids.len();
    h.ids.retain(|id| id != &session_id);
    if h.ids.len() != before {
        save_hidden(&app, &h)?;
        let _ = app.emit("data-changed", ());
    }
    Ok(())
}

#[tauri::command]
fn unhide_all(app: AppHandle) -> Result<(), String> {
    let h = HiddenFile::default();
    save_hidden(&app, &h)?;
    let _ = app.emit("data-changed", ());
    Ok(())
}

// ---- Hard delete -------------------------------------------------------
//
// Only inactive sessions can be hard-deleted — we refuse on active/idle to
// avoid yanking transcript files out from under a running Claude Code
// process. Two-step confirmation is enforced on the frontend; this command
// trusts the caller has confirmed.

#[derive(Debug, Serialize)]
struct DeleteReport {
    session_id: String,
    files_removed: usize,
    dirs_removed: usize,
}

#[tauri::command]
fn delete_session_files(session_id: String) -> Result<DeleteReport, String> {
    // Status guard: require inactive.
    let rows = api::list_sessions(None).map_err(|e| e.to_string())?;
    let row = rows
        .iter()
        .find(|r| r.session_id == session_id)
        .ok_or_else(|| format!("session {session_id} not found"))?;
    if !matches!(row.status, tokenscope::model::LiveStatus::Inactive) {
        return Err(format!(
            "refusing to delete: session is {:?}, only Inactive sessions are deletable",
            row.status
        ));
    }

    // Locate files by scanning ~/.claude/projects for matching basename.
    // Belt-and-suspenders: we don't trust the slug→cwd reverse mapping.
    let root = api::resolve_root(None).map_err(|e| e.to_string())?;
    let projects = root.join("projects");
    let mut files_removed = 0usize;
    let mut dirs_removed = 0usize;
    if let Ok(slugs) = std::fs::read_dir(&projects) {
        for slug_entry in slugs.flatten() {
            let slug_path = slug_entry.path();
            if !slug_path.is_dir() {
                continue;
            }
            // Transcript file: {session_id}.jsonl
            let transcript = slug_path.join(format!("{session_id}.jsonl"));
            if transcript.is_file() {
                if let Err(e) = std::fs::remove_file(&transcript) {
                    return Err(format!("remove {}: {e}", transcript.display()));
                }
                files_removed += 1;
            }
            // Subagent folder: {session_id}/
            let subdir = slug_path.join(&session_id);
            if subdir.is_dir() {
                if let Err(e) = std::fs::remove_dir_all(&subdir) {
                    return Err(format!("remove {}: {e}", subdir.display()));
                }
                dirs_removed += 1;
            }
        }
    }

    // Also drop the sessions/{pid}.json reference if any matches this id —
    // but only if pid is no longer alive (status was inactive). Walk the
    // sessions dir and remove json files whose `sessionId` matches.
    let sessions_dir = root.join("sessions");
    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&p) else { continue };
            let Ok(v): serde_json::Result<serde_json::Value> = serde_json::from_slice(&bytes)
            else {
                continue;
            };
            if v.get("sessionId").and_then(|s| s.as_str()) == Some(&session_id) {
                if let Err(e) = std::fs::remove_file(&p) {
                    return Err(format!("remove {}: {e}", p.display()));
                }
                files_removed += 1;
            }
        }
    }

    // Invalidate our own cache so the next list_sessions reflects reality.
    let cache_path = root.join(".tokenscope-cache.json");
    let _ = std::fs::remove_file(&cache_path);

    Ok(DeleteReport {
        session_id,
        files_removed,
        dirs_removed,
    })
}

// ---- Repo rollup -------------------------------------------------------

#[derive(Debug, Serialize)]
struct RepoRollup {
    repo: String,
    session_count: usize,
    live_count: usize,
    total_cost_usd: f64,
    total_tokens: u64,
    top_model: Option<String>,
}

#[tauri::command]
fn list_repo_rollups(app: AppHandle) -> Result<Vec<RepoRollup>, String> {
    use std::collections::HashMap;
    let rows = api::list_sessions(None).map_err(|e| e.to_string())?;
    let hidden: HashSet<String> = load_hidden(&app).ids.into_iter().collect();

    #[derive(Default)]
    struct Acc {
        session_count: usize,
        live_count: usize,
        cost: f64,
        tokens: u64,
        models: HashMap<String, u64>, // model → token volume, for top model
    }
    let mut map: HashMap<String, Acc> = HashMap::new();
    for r in &rows {
        if hidden.contains(&r.session_id) {
            continue;
        }
        let repo = r
            .cwd
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("-")
            .to_string();
        let a = map.entry(repo).or_default();
        a.session_count += 1;
        if matches!(
            r.status,
            tokenscope::model::LiveStatus::Active | tokenscope::model::LiveStatus::Idle
        ) {
            a.live_count += 1;
        }
        a.cost += r.cost_usd.unwrap_or(0.0);
        let row_tokens = r.tokens.input + r.tokens.output + r.tokens.cache_creation;
        a.tokens += row_tokens;
        if let Some(m) = &r.model {
            *a.models.entry(m.clone()).or_insert(0) += row_tokens;
        }
    }

    let mut out: Vec<RepoRollup> = map
        .into_iter()
        .map(|(repo, a)| {
            let top_model = a
                .models
                .iter()
                .max_by_key(|(_, v)| *v)
                .map(|(k, _)| k.clone());
            RepoRollup {
                repo,
                session_count: a.session_count,
                live_count: a.live_count,
                total_cost_usd: a.cost,
                total_tokens: a.tokens,
                top_model,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.total_cost_usd
            .partial_cmp(&a.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

// ---- Dashboard window --------------------------------------------------

#[tauri::command]
fn open_dashboard(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("dashboard") {
        let _ = win.show();
        let _ = win.set_focus();
        let _ = win.unminimize();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "dashboard", WebviewUrl::App("dashboard".into()))
        .title("Tokenscope — Dashboard")
        .inner_size(1000.0, 720.0)
        .min_inner_size(720.0, 480.0)
        .resizable(true)
        .visible(true)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn active_burn() -> Option<api::ActiveBurn> {
    api::list_blocks_enriched(None).ok()?.active
}

struct TrayState {
    title: String,
    tooltip: Option<String>,
}

// Minimal + alert breakthrough per design spec:
//   `$cost · $burn/hr` (burn hidden when < 0.5/hr → just `$cost`)
//   Prepend `⚠` when:
//     critical: any active/idle session at ≥ 90% context
//     warning:  budget > 0 and projected ≥ budget
//   Critical wins over warning. Tooltip names the cause.
fn tray_state(app: &AppHandle) -> TrayState {
    let burn = active_burn();
    let cost = burn.as_ref().map(|a| a.current_usd).unwrap_or(0.0);
    let rate = burn.as_ref().map(|a| a.burn_usd_per_hr).unwrap_or(0.0);
    let projected = burn.as_ref().map(|a| a.projected_block_usd).unwrap_or(0.0);

    let base = if rate >= 0.5 {
        format!("${cost:.2} · ${rate:.2}/hr")
    } else {
        format!("${cost:.2}")
    };

    // Critical: any live session ≥ 90% context.
    let sessions = api::list_sessions(None).unwrap_or_default();
    let worst = sessions
        .iter()
        .filter(|s| {
            matches!(
                s.status,
                tokenscope::model::LiveStatus::Active | tokenscope::model::LiveStatus::Idle
            )
        })
        .filter_map(|s| {
            if s.context_limit == 0 {
                return None;
            }
            let pct = s.context_tokens as f64 / s.context_limit as f64;
            Some((pct, s))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    if let Some((pct, s)) = worst {
        if pct >= 0.90 {
            let name = s
                .name
                .clone()
                .unwrap_or_else(|| s.session_id.chars().take(8).collect());
            return TrayState {
                title: format!("⚠ {base}"),
                tooltip: Some(format!(
                    "{name} — context {}%",
                    (pct * 100.0).round() as i64
                )),
            };
        }
    }

    let settings = load_settings(app);
    if settings.budget_window_usd > 0.0 && projected >= settings.budget_window_usd {
        return TrayState {
            title: format!("⚠ {base}"),
            tooltip: Some(format!(
                "projected ${projected:.2} ≥ ${:.2} budget",
                settings.budget_window_usd
            )),
        };
    }

    TrayState {
        title: base,
        tooltip: None,
    }
}

fn refresh_tray(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("main") {
        let state = tray_state(app);
        let _ = tray.set_title(Some(state.title));
        let _ = tray.set_tooltip(state.tooltip);
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
                .title("Tokenscope — projected over budget")
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
            .title("Tokenscope")
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

            let quit = MenuItem::with_id(app, "quit", "Quit Tokenscope", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            // 3-bar ascending bar-chart, rendered as a macOS template image so
            // the system tints it for light/dark menubar.
            let icon = Image::from_bytes(include_bytes!("../icons/tray/tray@2x.png"))?;

            let initial = tray_state(&app.handle());
            let tray = TrayIconBuilder::with_id("main")
                .icon(icon)
                .icon_as_template(true)
                .title(initial.title)
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
            let _ = tray.set_tooltip(initial.tooltip);

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
            list_hidden,
            hide_session,
            unhide_session,
            unhide_all,
            delete_session_files,
            list_repo_rollups,
            open_dashboard,
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
