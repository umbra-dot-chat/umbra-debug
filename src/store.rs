//! JSONL persistence for trace event sessions.
//!
//! Stores trace events as newline-delimited JSON in
//! `~/.umbra/debug-logs/`. Handles session rotation (max 20)
//! and provides query functions for CLI mode.

pub mod compare;
pub mod query;
pub mod replay;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Local;
use color_eyre::eyre::Result;

use crate::app::TraceEvent;

/// Maximum number of session files to retain.
const MAX_SESSIONS: usize = 20;

/// Get the log directory path (~/.umbra/debug-logs/).
pub fn log_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".umbra").join("debug-logs")
}

/// Session writer — appends trace events as JSONL to a session file.
#[allow(dead_code)]
pub struct SessionWriter {
    file: fs::File,
    pub path: PathBuf,
}

impl SessionWriter {
    /// Create a new session file and open it for writing.
    pub fn new() -> Result<Self> {
        let dir = log_dir();
        fs::create_dir_all(&dir)?;

        // Rotate old sessions
        rotate_sessions(&dir)?;

        let filename = format!(
            "session-{}.jsonl",
            Local::now().format("%Y-%m-%dT%H%M%S")
        );
        let path = dir.join(filename);
        let file = fs::File::create(&path)?;

        Ok(Self { file, path })
    }

    /// Write a single trace event as a JSON line.
    pub fn write_event(&mut self, event: &TraceEvent) -> Result<()> {
        let json = serde_json::to_string(event)?;
        writeln!(self.file, "{json}")?;
        Ok(())
    }
}

/// Load all events from a session file.
pub fn load_session(path: &Path) -> Result<Vec<TraceEvent>> {
    let content = fs::read_to_string(path)?;
    let mut events = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TraceEvent>(line) {
            Ok(ev) => events.push(ev),
            Err(e) => eprintln!("Skipping malformed line: {e}"),
        }
    }
    Ok(events)
}

/// List all session files sorted by modification time (newest first).
pub fn list_sessions(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut sessions: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    if dir.exists() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "jsonl")
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("session-"))
            {
                let mtime = entry.metadata()?.modified()?;
                sessions.push((path, mtime));
            }
        }
    }

    sessions.sort_by(|a, b| b.1.cmp(&a.1));
    Ok(sessions.into_iter().map(|(p, _)| p).collect())
}

/// Find the latest crash report file.
pub fn find_latest_crash(dir: &Path) -> Result<Option<PathBuf>> {
    let mut crashes: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    if dir.exists() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md")
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("crash-"))
            {
                let mtime = entry.metadata()?.modified()?;
                crashes.push((path, mtime));
            }
        }
    }

    crashes.sort_by(|a, b| b.1.cmp(&a.1));
    Ok(crashes.into_iter().next().map(|(p, _)| p))
}

/// Remove oldest session files if we exceed MAX_SESSIONS.
fn rotate_sessions(dir: &Path) -> Result<()> {
    let sessions = list_sessions(dir)?;
    if sessions.len() >= MAX_SESSIONS {
        // Delete the oldest sessions beyond the limit
        for path in sessions.iter().skip(MAX_SESSIONS - 1) {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}
