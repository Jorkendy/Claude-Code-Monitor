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
    /// 0 disables the alert. Used only when plan == "api".
    budget_window_usd: f64,
    /// "api" | "pro" | "max-5x" | "max-20x". API mode is per-token cost;
    /// subscription modes track messages-vs-quota in the same 5h window.
    #[serde(default = "default_plan")]
    plan: String,
    /// % of estimated 5h message quota that triggers a rate-limit warning.
    /// Used only for subscription plans. 0 disables.
    #[serde(default = "default_rate_warn")]
    rate_limit_warn_pct: f64,
    /// % of context window that triggers the critical tray warning
    /// (plan-agnostic). 0 disables.
    #[serde(default = "default_context_warn")]
    context_warn_pct: f64,
    /// User-supplied quota override (messages per 5h window). Anthropic
    /// doesn't publish exact numbers; this lets a user lock in what they've
    /// actually measured. Subscription plans only.
    #[serde(default)]
    custom_quota: Option<u64>,
    /// "system" | "light" | "dark". `system` follows OS via the existing
    /// `prefers-color-scheme` rules in tokens.css.
    #[serde(default = "default_theme")]
    theme: String,
    /// True until the user has explicitly picked a plan. Drives the
    /// first-run modal.
    #[serde(default = "default_true")]
    first_run: bool,
    /// Show app icon in the macOS Dock + Cmd+Tab list. False (default)
    /// = pure menubar app (Accessory). Toggled at runtime.
    #[serde(default)]
    show_in_dock: bool,
}

fn default_plan() -> String {
    "api".to_string()
}
fn default_rate_warn() -> f64 {
    90.0
}
fn default_context_warn() -> f64 {
    90.0
}
fn default_theme() -> String {
    "system".to_string()
}
fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            budget_window_usd: 5.0,
            plan: default_plan(),
            rate_limit_warn_pct: default_rate_warn(),
            context_warn_pct: default_context_warn(),
            custom_quota: None,
            theme: default_theme(),
            first_run: true,
            show_in_dock: false,
        }
    }
}

/// Community-estimated message quota per 5h window for each plan.
/// Returns None for "api" (no quota; cost-based).
fn quota_for(plan: &str) -> Option<u64> {
    match plan {
        "pro" => Some(45),
        "max-5x" => Some(225),
        "max-20x" => Some(900),
        _ => None,
    }
}

/// Effective quota: user override wins over the community estimate when set.
/// Returns None for "api" (no quota; cost-based).
fn effective_quota(s: &Settings) -> Option<u64> {
    let base = quota_for(&s.plan)?;
    Some(s.custom_quota.filter(|&q| q > 0).unwrap_or(base))
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
    // Live-apply the dock policy so the user sees the change without
    // restarting. NSApp `setActivationPolicy:` is safe to call repeatedly.
    #[cfg(target_os = "macos")]
    {
        let policy = if settings.show_in_dock {
            ActivationPolicy::Regular
        } else {
            ActivationPolicy::Accessory
        };
        let _ = app.set_activation_policy(policy);
    }
    // Notify other windows so the dashboard picks up theme / plan changes
    // without waiting for the next filesystem-watcher tick.
    let _ = app.emit("data-changed", ());
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

// Drops hidden ids whose backing session is gone. Caller passes the current
// alive session id list; we keep only the intersection. No emit — caller
// already has fresh data and would double-load on data-changed.
#[tauri::command]
fn prune_hidden(app: AppHandle, alive_ids: Vec<String>) -> Result<(), String> {
    let mut h = load_hidden(&app);
    let alive: std::collections::HashSet<String> = alive_ids.into_iter().collect();
    let before = h.ids.len();
    h.ids.retain(|id| alive.contains(id));
    if h.ids.len() != before {
        save_hidden(&app, &h)?;
    }
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

#[derive(Default)]
struct ActiveSnapshot {
    cost: f64,
    rate: f64,
    projected: f64,
    block_start_ms: i64,
    message_count: u64,
}

// Single read of the enriched blocks: avoids two separate scans when both
// tray formatting and alerting want the active-block fields.
fn active_snapshot() -> ActiveSnapshot {
    let Ok(enriched) = api::list_blocks_enriched(None) else {
        return ActiveSnapshot::default();
    };
    let active_msgs = enriched
        .blocks
        .iter()
        .find(|b| b.is_active)
        .map(|b| b.message_count as u64)
        .unwrap_or(0);
    let Some(burn) = enriched.active else {
        return ActiveSnapshot::default();
    };
    ActiveSnapshot {
        cost: burn.current_usd,
        rate: burn.burn_usd_per_hr,
        projected: burn.projected_block_usd,
        block_start_ms: burn.block_start_ms,
        message_count: active_msgs,
    }
}

struct TrayState {
    title: String,
    tooltip: Option<String>,
}

// Minimal + alert breakthrough per design spec:
//   API mode:          `$cost · $burn/hr` (burn hidden when < 0.5/hr)
//   Subscription mode: `N msgs · NN%`
//   Prepend `⚠` when:
//     critical: any active/idle session at ≥ 90% context (both modes)
//     warning:  budget threshold (API) or rate-limit % (subscription)
//   Critical wins over warning. Tooltip names the cause.
fn tray_state(app: &AppHandle) -> TrayState {
    let snap = active_snapshot();
    let settings = load_settings(app);
    let quota = effective_quota(&settings);

    let base = if let Some(q) = quota {
        let pct = if q > 0 {
            (snap.message_count as f64 / q as f64 * 100.0).round() as i64
        } else {
            0
        };
        format!("{} msgs · {}%", snap.message_count, pct)
    } else if snap.rate >= 0.5 {
        format!("${:.2} · ${:.2}/hr", snap.cost, snap.rate)
    } else {
        format!("${:.2}", snap.cost)
    };

    // Critical: any session worth warning about at ≥ `context_warn_pct`.
    // Active always counts; Idle only when it last updated within 30 min —
    // a session left open from yesterday at 97% is not actionable now.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    const IDLE_RECENT_MS: i64 = 30 * 60 * 1000;
    let sessions = api::list_sessions(None).unwrap_or_default();
    let worst = sessions
        .iter()
        .filter(|s| match s.status {
            tokenscope::model::LiveStatus::Active => true,
            tokenscope::model::LiveStatus::Idle => s
                .updated_at_ms
                .map(|u| now_ms - u <= IDLE_RECENT_MS)
                .unwrap_or(false),
            _ => false,
        })
        .filter_map(|s| {
            if s.context_limit == 0 {
                return None;
            }
            let pct = s.context_tokens as f64 / s.context_limit as f64;
            Some((pct, s))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    if settings.context_warn_pct > 0.0 {
        if let Some((pct, s)) = worst {
            if pct * 100.0 >= settings.context_warn_pct {
                let name = s
                    .name
                    .clone()
                    .unwrap_or_else(|| s.session_id.chars().take(8).collect());
                let ctx_pct = (pct * 100.0).round() as i64;
                return TrayState {
                    title: format!("⚠ ctx {}% · {}", ctx_pct, base),
                    tooltip: Some(format!("{name} — context {}%", ctx_pct)),
                };
            }
        }
    }

    // Warning: plan-specific budget vs projected (API) or rate-limit % (subs).
    if let Some(q) = quota {
        if settings.rate_limit_warn_pct > 0.0 && q > 0 {
            let pct = snap.message_count as f64 / q as f64 * 100.0;
            if pct >= settings.rate_limit_warn_pct {
                return TrayState {
                    title: format!("⚠ {base}"),
                    tooltip: Some(format!(
                        "{:.0}% of estimated {q}-msg quota used",
                        pct
                    )),
                };
            }
        }
    } else if settings.budget_window_usd > 0.0 && snap.projected >= settings.budget_window_usd {
        return TrayState {
            title: format!("⚠ {base}"),
            tooltip: Some(format!(
                "projected ${:.2} ≥ ${:.2} budget",
                snap.projected, settings.budget_window_usd
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
    let snap = active_snapshot();
    if snap.block_start_ms == 0 {
        eprintln!("[alert] no active block");
        return;
    }

    // Re-arm fire-once-per-block latches when the 5h window rolls over.
    {
        let mut last_block = PROJ_ALERTED_BLOCK_START.lock().unwrap();
        if *last_block != Some(snap.block_start_ms) {
            *last_block = None;
        }
    }

    if let Some(q) = effective_quota(&settings) {
        // Subscription plan: rate-limit warning at % of estimated quota.
        if settings.rate_limit_warn_pct <= 0.0 || q == 0 {
            eprintln!("[alert] rate-limit disabled (threshold=0)");
            return;
        }
        let pct = snap.message_count as f64 / q as f64 * 100.0;
        eprintln!(
            "[alert] subs plan={} msgs={}/{q} ({:.0}%) threshold={:.0}%",
            settings.plan, snap.message_count, pct, settings.rate_limit_warn_pct
        );
        let mut fired = ALERT_FIRED_FOR_CURRENT_CROSSING.lock().unwrap();
        if pct < settings.rate_limit_warn_pct {
            *fired = false;
            return;
        }
        if !*fired {
            let res = app
                .notification()
                .builder()
                .title("Tokenscope — approaching rate limit")
                .body(format!(
                    "{} of ~{q} messages used in this 5h window ({:.0}%)",
                    snap.message_count, pct
                ))
                .show();
            eprintln!("[alert] rate-limit notification => {res:?}");
            *fired = true;
        }
        return;
    }

    // API plan: budget threshold.
    if settings.budget_window_usd <= 0.0 {
        eprintln!("[alert] budget disabled (threshold=0)");
        return;
    }
    eprintln!(
        "[alert] api cost=${:.2} proj=${:.2} threshold=${:.2}",
        snap.cost, snap.projected, settings.budget_window_usd
    );
    // Proactive: projected to cross but actual hasn't yet → tell user once
    // per block, so they can pause before they're surprised.
    if snap.projected >= settings.budget_window_usd
        && snap.cost < settings.budget_window_usd
    {
        let mut last_block = PROJ_ALERTED_BLOCK_START.lock().unwrap();
        if *last_block != Some(snap.block_start_ms) {
            let res = app
                .notification()
                .builder()
                .title("Tokenscope — projected over budget")
                .body(format!(
                    "Block trending to ${:.2} by reset (threshold ${:.2}, burn ${:.0}/hr)",
                    snap.projected, settings.budget_window_usd, snap.rate
                ))
                .show();
            eprintln!("[alert] projection notification => {res:?}");
            *last_block = Some(snap.block_start_ms);
        }
    }
    let mut fired = ALERT_FIRED_FOR_CURRENT_CROSSING.lock().unwrap();
    let mut last = LAST_ALERT.lock().unwrap();
    if snap.cost < settings.budget_window_usd {
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
                "Current 5h block at ${:.2} (threshold ${:.2})",
                snap.cost, settings.budget_window_usd
            ))
            .show();
        eprintln!("[alert] notification show => {res:?}");
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

// ---- App info & updates ------------------------------------------------
//
// `check_update` polls the GitHub Releases API for the latest tag and
// compares with the current binary version. `download_update` streams the
// matching `.dmg` asset to ~/Downloads and emits progress events; on
// completion the caller can ask `open_in_finder` to launch the DMG so the
// user installs in the normal mac way. We deliberately don't auto-swap
// the .app — that needs code signing + Sparkle, which the prerelease
// distribution doesn't have.

const REPO: &str = "Jorkendy/Claude-Code-Monitor";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize)]
struct AppInfo {
    version: &'static str,
    repo_url: String,
    release_url: String,
    latest_release_url: String,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: APP_VERSION,
        repo_url: format!("https://github.com/{REPO}"),
        release_url: format!("https://github.com/{REPO}/releases/tag/v{APP_VERSION}"),
        latest_release_url: format!("https://github.com/{REPO}/releases/latest"),
    }
}

#[derive(Debug, Serialize)]
struct UpdateInfo {
    current_version: String,
    latest_version: String,
    has_update: bool,
    release_notes: String,
    release_url: String,
    /// URL of the aarch64 `.dmg` asset, if any. Frontend disables Download
    /// when this is None.
    dmg_url: Option<String>,
    dmg_size: Option<u64>,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("Tokenscope/{APP_VERSION}"))
        .timeout(std::time::Duration::from_secs(15))
        .build()
}

/// Strip a leading `v` so `v0.1.2` compares equal to `0.1.2`.
fn normalize_version(v: &str) -> &str {
    v.strip_prefix('v').unwrap_or(v).trim()
}

#[tauri::command]
async fn check_update() -> Result<UpdateInfo, String> {
    let client = http_client().map_err(|e| e.to_string())?;
    // Use /releases (plural) and pick the newest non-draft so we still find
    // updates when the latest published release is marked prerelease — the
    // `/releases/latest` endpoint hides prereleases entirely.
    let resp = client
        .get(format!("https://api.github.com/repos/{REPO}/releases?per_page=10"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
    }
    let releases: Vec<GhRelease> = resp.json().await.map_err(|e| e.to_string())?;
    let release = releases
        .into_iter()
        .find(|r| !r.tag_name.is_empty())
        .ok_or_else(|| "no releases found".to_string())?;

    let latest = normalize_version(&release.tag_name).to_string();
    let current = APP_VERSION.to_string();
    let has_update = latest != current;
    let dmg = release
        .assets
        .iter()
        .find(|a| a.name.ends_with("_aarch64.dmg") || a.name.ends_with(".dmg"));

    Ok(UpdateInfo {
        current_version: current,
        latest_version: latest,
        has_update,
        release_notes: release.body,
        release_url: release.html_url,
        dmg_url: dmg.map(|a| a.browser_download_url.clone()),
        dmg_size: dmg.map(|a| a.size),
    })
}

#[derive(Clone, Serialize)]
struct DownloadProgress {
    downloaded: u64,
    total: u64,
}

#[derive(Debug, Serialize)]
struct DownloadResult {
    path: String,
}

#[tauri::command]
async fn download_update(
    app: AppHandle,
    url: String,
) -> Result<DownloadResult, String> {
    use futures_util::StreamExt;
    use std::io::Write;

    let downloads = dirs::download_dir().ok_or("no Downloads dir")?;
    let file_name = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Tokenscope_update.dmg");
    let dest = downloads.join(file_name);

    let client = http_client().map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let total = resp.content_length().unwrap_or(0);

    let mut file = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    let mut last_emit = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        downloaded += bytes.len() as u64;
        // Throttle event emission so we don't flood the IPC bridge on a
        // fast download. ~10/sec is plenty for a smooth progress bar.
        if last_emit.elapsed() >= std::time::Duration::from_millis(100) {
            let _ = app.emit(
                "update-progress",
                DownloadProgress { downloaded, total },
            );
            last_emit = std::time::Instant::now();
        }
    }
    let _ = app.emit(
        "update-progress",
        DownloadProgress {
            downloaded,
            total: total.max(downloaded),
        },
    );
    file.flush().map_err(|e| e.to_string())?;

    Ok(DownloadResult {
        path: dest.to_string_lossy().to_string(),
    })
}

#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    // Plain `open` so the user gets the standard Finder "Verify → Install"
    // flow for a DMG. Avoids needing the Tauri shell plugin permission.
    std::process::Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Apply the saved dock policy on startup. Accessory hides the
            // app from the Dock + Cmd+Tab (menubar-only); Regular shows it.
            #[cfg(target_os = "macos")]
            {
                let s = load_settings(&app.handle());
                let policy = if s.show_in_dock {
                    ActivationPolicy::Regular
                } else {
                    ActivationPolicy::Accessory
                };
                let _ = app.set_activation_policy(policy);
            }

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
            // focus before the timer fires, we skip the hide. Only applies
            // to the popover ("main"); the dashboard is a real window the
            // user expects to keep open alongside other apps.
            WindowEvent::Focused(false) if window.label() == "main" => {
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
            prune_hidden,
            delete_session_files,
            list_repo_rollups,
            open_dashboard,
            app_info,
            check_update,
            download_update,
            open_path,
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
