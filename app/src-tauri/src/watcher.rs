// File-watcher: notify-rs on ~/.claude/sessions/ and ~/.claude/projects/.
// We debounce by draining the channel for 2s after each event, so a burst of
// writes from an active session triggers exactly one refresh.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};

const DEBOUNCE: Duration = Duration::from_secs(2);

pub fn spawn(app: AppHandle, root: PathBuf, on_change: impl Fn(&AppHandle) + Send + 'static) {
    std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel::<()>();
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("watcher init failed: {e}");
                return;
            }
        };

        for sub in ["sessions", "projects"] {
            let path = root.join(sub);
            if path.is_dir() {
                if let Err(e) = watcher.watch(&path, RecursiveMode::Recursive) {
                    eprintln!("watch {} failed: {e}", path.display());
                }
            }
        }

        loop {
            if rx.recv().is_err() {
                break;
            }
            while rx.recv_timeout(DEBOUNCE).is_ok() {}
            on_change(&app);
            let _ = app.emit("data-changed", ());
        }
    });
}
