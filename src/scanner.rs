// Scanner: discover files under ~/.claude/.
//
// Layout (verified on real data):
//   sessions/{pid}.json                                 -> live metadata
//   projects/{slug}/{sessionId}.jsonl                   -> session transcript
//   projects/{slug}/{sessionId}/subagents/agent-*.jsonl -> subagent transcripts
//   projects/{slug}/{sessionId}/subagents/agent-*.meta.json -> agent type/desc (skipped here)
//   projects/{slug}/{sessionId}/tool-results/...        -> not relevant (skipped)
//   projects/{slug}/memory/                              -> not a session (skipped)
//
// Read-only. Any I/O error on a single path is logged via the caller's discretion
// (here we simply propagate from top-level dirs and ignore at leaf level).

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct ScanResult {
    pub session_files: Vec<PathBuf>,
    pub transcript_files: Vec<PathBuf>,
    pub subagent_files: Vec<PathBuf>,
}

pub fn scan(root: &Path) -> Result<ScanResult> {
    let mut result = ScanResult::default();

    let sessions_dir = root.join("sessions");
    if sessions_dir.is_dir() {
        for entry in fs::read_dir(&sessions_dir)? {
            let Ok(entry) = entry else { continue };
            let p = entry.path();
            if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("json") {
                result.session_files.push(p);
            }
        }
    }

    let projects_dir = root.join("projects");
    if projects_dir.is_dir() {
        for slug in fs::read_dir(&projects_dir)? {
            let Ok(slug) = slug else { continue };
            let slug_path = slug.path();
            if !slug_path.is_dir() {
                continue;
            }
            for child in fs::read_dir(&slug_path)? {
                let Ok(child) = child else { continue };
                let c = child.path();
                if c.is_file() && c.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    result.transcript_files.push(c);
                } else if c.is_dir() {
                    let subs = c.join("subagents");
                    if subs.is_dir() {
                        if let Ok(rd) = fs::read_dir(&subs) {
                            for sub in rd.flatten() {
                                let sp = sub.path();
                                if !sp.is_file() {
                                    continue;
                                }
                                let name = sp.file_name().and_then(|n| n.to_str()).unwrap_or("");
                                if name.starts_with("agent-") && name.ends_with(".jsonl") {
                                    result.subagent_files.push(sp);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(result)
}
