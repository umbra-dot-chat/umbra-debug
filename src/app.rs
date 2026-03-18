//! Application state, tab management, and event routing.
//!
//! The `App` struct owns all trace events, per-client state,
//! computed statistics, and UI mode (active tab, filter, scroll).

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::server::WsEvent;
use crate::store::compare::SessionComparison;
use crate::store::replay::{ReplaySession, ReplaySpeed};
use crate::store::SessionWriter;
use crate::ui;
use crate::ui::breakpoint;
use crate::ui::breakpoint::Breakpoint;
use crate::ui::deps_tab::DepsState;

/// Maximum events kept in memory (circular buffer behavior).
const MAX_EVENTS: usize = 100_000;

/// Maximum log entries kept in the Log tab.
const MAX_LOG_ENTRIES: usize = 2_000;

/// A single trace event received from the browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Monotonically increasing sequence number.
    pub seq: u64,
    /// High-resolution timestamp (performance.now() in ms).
    pub ts: f64,
    /// Event category.
    pub cat: String,
    /// Function name.
    #[serde(rename = "fn")]
    pub func: String,
    /// Argument size in bytes.
    #[serde(rename = "argBytes", default)]
    pub arg_bytes: u64,
    /// Optional argument preview string.
    #[serde(rename = "argPreview", default)]
    pub arg_preview: Option<String>,
    /// Duration in milliseconds.
    #[serde(rename = "durMs", default)]
    pub dur_ms: f64,
    /// WASM memory before call.
    #[serde(rename = "memBefore", default)]
    pub mem_before: i64,
    /// WASM memory after call.
    #[serde(rename = "memAfter", default)]
    pub mem_after: i64,
    /// Memory growth (memAfter - memBefore).
    #[serde(rename = "memGrowth", default)]
    pub mem_growth: i64,
    /// Parent SQL caller context.
    #[serde(rename = "sqlContext", default)]
    pub sql_context: Option<String>,
    /// Client identifier.
    #[serde(rename = "clientId", default)]
    pub client_id: String,
    /// Error message if this event represents a failure.
    #[serde(default)]
    pub err: Option<String>,
    /// Optional metadata (present on log entries and vitals heartbeats).
    #[serde(default)]
    pub meta: Option<TraceMeta>,
}

/// Metadata attached to trace events from the browser debug bridge.
///
/// When `level` is non-empty, this is a log entry.
/// When `level` is empty/absent, this is a vitals heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TraceMeta {
    /// Log level: trace, debug, info, warn, error, fatal.
    #[serde(default)]
    pub level: String,
    /// Source hook/module name (e.g. "useMessages").
    #[serde(default)]
    pub src: String,
    /// Serialized data payload.
    #[serde(default)]
    pub data: String,
    /// Stack trace (if available).
    #[serde(default)]
    pub stack: String,
}

/// A parsed log entry for the Log tab.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Browser timestamp (performance.now() in ms).
    pub timestamp: f64,
    /// Log level (trace, debug, info, warn, error, fatal).
    pub level: String,
    /// Event category (e.g. "messages", "sync", "auth").
    pub category: String,
    /// Source hook/module (e.g. "useMessages").
    pub source: String,
    /// Log message (from the fn field).
    pub message: String,
    /// Data payload.
    pub data: String,
}

/// All log categories for cycling the category filter.
pub const LOG_CATEGORIES: &[&str] = &[
    "render",
    "service",
    "network",
    "state",
    "lifecycle",
    "perf",
    "conversations",
    "messages",
    "friends",
    "sync",
    "auth",
    "plugins",
    "call",
    "groups",
    "community",
];

/// All log levels in ascending severity order for cycling.
pub const LOG_LEVELS: &[&str] = &[
    "trace", "debug", "info", "warn", "error", "fatal",
];

/// Connected client info from the hello handshake.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ClientInfo {
    pub client_id: String,
    pub user_agent: String,
    pub device_memory: f64,
    pub connected_at: Instant,
}

/// Active tab in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    All,
    Wasm,
    Sql,
    Net,
    Mem,
    Err,
    Analysis,
    Browser,
    Log,
    Compare,
    Replay,
    Deps,
}

impl Tab {
    pub const ALL: [Tab; 13] = [
        Tab::Dashboard,
        Tab::All,
        Tab::Wasm,
        Tab::Sql,
        Tab::Net,
        Tab::Mem,
        Tab::Err,
        Tab::Analysis,
        Tab::Browser,
        Tab::Log,
        Tab::Compare,
        Tab::Replay,
        Tab::Deps,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => "Dash",
            Tab::All => "All",
            Tab::Wasm => "WASM",
            Tab::Sql => "SQL",
            Tab::Net => "Net",
            Tab::Mem => "Mem",
            Tab::Err => "Err",
            Tab::Analysis => "Analysis",
            Tab::Browser => "Browser",
            Tab::Log => "Log",
            Tab::Compare => "Compare",
            Tab::Replay => "Replay",
            Tab::Deps => "Deps",
        }
    }

    pub fn next(self) -> Tab {
        let idx = Tab::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Tab::ALL[(idx + 1) % Tab::ALL.len()]
    }

    pub fn prev(self) -> Tab {
        let idx = Tab::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Tab::ALL[(idx + Tab::ALL.len() - 1) % Tab::ALL.len()]
    }
}

/// Per-function aggregated stats (for WASM and Analysis tabs).
#[derive(Debug, Clone, Default)]
pub struct FuncStats {
    pub call_count: u64,
    pub total_dur_ms: f64,
    pub total_mem_growth: i64,
    /// Recent call timestamps for calls/sec calculation.
    pub recent_calls: Vec<f64>,
}

/// Application state.
pub struct App {
    /// All trace events this session.
    pub events: Vec<TraceEvent>,
    /// Active tab.
    pub tab: Tab,
    /// Whether event ingestion is paused.
    pub paused: bool,
    /// Scroll offset for scrollable views.
    pub scroll_offset: usize,
    /// Compiled regex filter (applied to All tab).
    pub filter: Option<Regex>,
    /// Current filter text being typed.
    pub filter_input: String,
    /// Whether the filter input bar is active.
    pub filter_mode: bool,
    /// Connected clients.
    pub clients: HashMap<String, ClientInfo>,
    /// Session start time.
    pub session_start: Instant,
    /// Events per second (calculated from recent events).
    pub events_per_sec: f64,
    /// Memory graph window in seconds (adjustable).
    pub mem_graph_window_secs: u16,
    /// Per-function aggregated stats.
    pub func_stats: HashMap<String, FuncStats>,
    /// Whether the app should quit.
    pub should_quit: bool,
    /// Session JSONL writer.
    pub session_writer: Option<SessionWriter>,
    /// Whether auto-scroll is active (user hasn't scrolled up).
    pub auto_scroll: bool,
    /// Event count at last tick (for events/sec calculation).
    events_at_last_tick: usize,
    /// Tick counter for events/sec smoothing.
    tick_count: u64,

    // -- Log tab state --

    /// Log entries for the Log tab (capped at MAX_LOG_ENTRIES).
    pub log_entries: VecDeque<LogEntry>,
    /// Filter by log level (only show entries >= this level).
    pub log_level_filter: Option<String>,
    /// Filter by category (exact match).
    pub log_category_filter: Option<String>,
    /// Filter by source (exact match).
    pub log_source_filter: Option<String>,
    /// Scroll offset for the Log tab.
    pub log_scroll_offset: usize,
    /// Whether the Log tab auto-scrolls to latest entries.
    pub log_auto_scroll: bool,
    /// Whether the Log tab is in source-filter input mode.
    pub log_source_input_mode: bool,
    /// Current source filter text being typed.
    pub log_source_input: String,

    // -- Compare tab state --

    /// Loaded session comparison (populated via `C` key or `--compare`).
    pub comparison: Option<SessionComparison>,

    // -- Replay tab state --

    /// Interactive replay session for time-travel debugging.
    pub replay_session: Option<ReplaySession>,
    /// Whether the replay filter input mode is active.
    pub replay_filter_mode: bool,
    /// Current replay filter text being typed.
    pub replay_filter_input: String,

    // -- Breakpoint state --

    /// Active breakpoints that can pause event ingestion.
    pub breakpoints: Vec<Breakpoint>,
    /// Whether event ingestion is paused by a breakpoint.
    pub bp_paused: bool,
    /// Reason why the breakpoint paused (human-readable).
    pub bp_pause_reason: Option<String>,
    /// Whether the breakpoint input dialog is visible.
    pub bp_input_mode: bool,
    /// Current breakpoint input text being typed.
    pub bp_input: String,
    /// Event index where the breakpoint pause occurred.
    pub bp_pause_index: Option<usize>,
    /// Whether to advance exactly one event (step mode).
    pub bp_step_one: bool,

    // -- Deps tab state --

    /// Dependency graph state for event chain visualization.
    pub deps_state: Option<DepsState>,
}

impl App {
    /// Create a new App with default state.
    pub fn new() -> Self {
        let session_writer = SessionWriter::new()
            .map_err(|e| eprintln!("Failed to create session writer: {e}"))
            .ok();

        Self {
            events: Vec::with_capacity(1024),
            tab: Tab::Dashboard,
            paused: false,
            scroll_offset: 0,
            filter: None,
            filter_input: String::new(),
            filter_mode: false,
            clients: HashMap::new(),
            session_start: Instant::now(),
            events_per_sec: 0.0,
            mem_graph_window_secs: 60,
            func_stats: HashMap::new(),
            should_quit: false,
            session_writer,
            auto_scroll: true,
            events_at_last_tick: 0,
            tick_count: 0,
            log_entries: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            log_level_filter: None,
            log_category_filter: None,
            log_source_filter: None,
            log_scroll_offset: 0,
            log_auto_scroll: true,
            log_source_input_mode: false,
            log_source_input: String::new(),
            comparison: None,
            replay_session: None,
            replay_filter_mode: false,
            replay_filter_input: String::new(),
            breakpoints: Vec::new(),
            bp_paused: false,
            bp_pause_reason: None,
            bp_input_mode: false,
            bp_input: String::new(),
            bp_pause_index: None,
            bp_step_one: false,
            deps_state: None,
        }
    }

    /// Handle a key press event.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // In filter input mode, route keys to filter editing
        if self.filter_mode {
            match code {
                KeyCode::Esc => {
                    self.filter_mode = false;
                    self.filter_input.clear();
                    self.filter = None;
                }
                KeyCode::Enter => {
                    self.filter_mode = false;
                    if self.filter_input.is_empty() {
                        self.filter = None;
                    } else {
                        match Regex::new(&self.filter_input) {
                            Ok(re) => self.filter = Some(re),
                            Err(_) => {
                                // Invalid regex — clear filter
                                self.filter = None;
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    self.filter_input.pop();
                }
                KeyCode::Char(c) => {
                    self.filter_input.push(c);
                }
                _ => {}
            }
            return;
        }

        // In log source filter input mode
        if self.log_source_input_mode {
            match code {
                KeyCode::Esc => {
                    self.log_source_input_mode = false;
                    self.log_source_input.clear();
                }
                KeyCode::Enter => {
                    self.log_source_input_mode = false;
                    if self.log_source_input.is_empty() {
                        self.log_source_filter = None;
                    } else {
                        self.log_source_filter = Some(self.log_source_input.clone());
                    }
                }
                KeyCode::Backspace => {
                    self.log_source_input.pop();
                }
                KeyCode::Char(c) => {
                    self.log_source_input.push(c);
                }
                _ => {}
            }
            return;
        }

        // Replay filter input mode
        if self.replay_filter_mode {
            match code {
                KeyCode::Esc => {
                    self.replay_filter_mode = false;
                    self.replay_filter_input.clear();
                    if let Some(ref mut session) = self.replay_session {
                        session.set_filter(None);
                    }
                }
                KeyCode::Enter => {
                    self.replay_filter_mode = false;
                    if let Some(ref mut session) = self.replay_session {
                        if self.replay_filter_input.is_empty() {
                            session.set_filter(None);
                        } else {
                            session.set_filter(Some(self.replay_filter_input.clone()));
                        }
                    }
                }
                KeyCode::Backspace => {
                    self.replay_filter_input.pop();
                }
                KeyCode::Char(c) => {
                    self.replay_filter_input.push(c);
                }
                _ => {}
            }
            return;
        }

        // In breakpoint input mode
        if self.bp_input_mode {
            match code {
                KeyCode::Esc => {
                    self.bp_input_mode = false;
                    self.bp_input.clear();
                }
                KeyCode::Enter => {
                    self.bp_input_mode = false;
                    if let Some(bp) = breakpoint::parse_condition(&self.bp_input) {
                        self.breakpoints.push(bp);
                    }
                    self.bp_input.clear();
                }
                KeyCode::Backspace => {
                    self.bp_input.pop();
                }
                KeyCode::Char(c) => {
                    self.bp_input.push(c);
                }
                _ => {}
            }
            return;
        }

        // Replay tab-specific keybindings
        if self.tab == Tab::Replay {
            if let Some(ref mut session) = self.replay_session {
                match code {
                    KeyCode::Right => {
                        session.step_forward();
                        return;
                    }
                    KeyCode::Left => {
                        session.step_backward();
                        return;
                    }
                    KeyCode::Char('1') => {
                        session.set_speed(ReplaySpeed::X1);
                        return;
                    }
                    KeyCode::Char('2') => {
                        session.set_speed(ReplaySpeed::X2);
                        return;
                    }
                    KeyCode::Char('5') => {
                        session.set_speed(ReplaySpeed::X5);
                        return;
                    }
                    KeyCode::Char('0') => {
                        session.set_speed(ReplaySpeed::X10);
                        return;
                    }
                    KeyCode::Char('p') => {
                        let new_speed = if session.speed == ReplaySpeed::Paused {
                            ReplaySpeed::X1
                        } else {
                            ReplaySpeed::Paused
                        };
                        session.set_speed(new_speed);
                        return;
                    }
                    KeyCode::Char('m') => {
                        session.toggle_bookmark();
                        return;
                    }
                    KeyCode::Char('n') => {
                        session.goto_next_bookmark();
                        return;
                    }
                    KeyCode::Char('N') => {
                        session.goto_prev_bookmark();
                        return;
                    }
                    KeyCode::Enter => {
                        session.detail_expanded = !session.detail_expanded;
                        return;
                    }
                    KeyCode::Home => {
                        session.goto_start();
                        return;
                    }
                    KeyCode::End => {
                        session.goto_end();
                        return;
                    }
                    _ => {} // Fall through to global keys
                }
            }

            // 'l' loads latest session (works even with no session)
            if code == KeyCode::Char('l') {
                self.load_replay_from_latest();
                return;
            }

            // '/' enters replay filter mode
            if code == KeyCode::Char('/') {
                self.replay_filter_mode = true;
                self.replay_filter_input.clear();
                return;
            }
        }

        // Deps tab-specific keybindings
        if self.tab == Tab::Deps {
            match code {
                KeyCode::Enter => {
                    if let Some(ref mut deps) = self.deps_state {
                        deps.toggle_selected();
                    }
                    return;
                }
                KeyCode::Char('f') => {
                    if let Some(ref mut deps) = self.deps_state {
                        deps.filter_subtree();
                    }
                    return;
                }
                KeyCode::Char('t') => {
                    if let Some(ref mut deps) = self.deps_state {
                        deps.show_timing = !deps.show_timing;
                    }
                    return;
                }
                KeyCode::Char('c') if !modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(ref mut deps) = self.deps_state {
                        deps.show_counts = !deps.show_counts;
                    }
                    return;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    if let Some(ref mut deps) = self.deps_state {
                        deps.select_down();
                    }
                    return;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    if let Some(ref mut deps) = self.deps_state {
                        deps.select_up();
                    }
                    return;
                }
                KeyCode::Char('r') => {
                    self.build_deps_graph();
                    return;
                }
                _ => {} // Fall through to global keys
            }
        }

        // Log tab-specific keybindings
        if self.tab == Tab::Log {
            match code {
                KeyCode::Char('c') if !modifiers.contains(KeyModifiers::CONTROL) => {
                    self.cycle_log_category_filter();
                    return;
                }
                KeyCode::Char('l') => {
                    self.cycle_log_level_filter();
                    return;
                }
                KeyCode::Char('s') => {
                    if self.log_source_filter.is_some() {
                        // Clear existing source filter
                        self.log_source_filter = None;
                    } else {
                        // Enter source filter input mode
                        self.log_source_input_mode = true;
                        self.log_source_input.clear();
                    }
                    return;
                }
                KeyCode::Char('x') => {
                    self.log_entries.clear();
                    self.log_scroll_offset = 0;
                    self.log_auto_scroll = true;
                    return;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.log_scroll_offset = self.log_scroll_offset.saturating_add(1);
                    self.log_auto_scroll = false;
                    return;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.log_scroll_offset = self.log_scroll_offset.saturating_sub(1);
                    if self.log_scroll_offset == 0 {
                        self.log_auto_scroll = true;
                    }
                    return;
                }
                KeyCode::Char('G') => {
                    self.log_auto_scroll = true;
                    self.log_scroll_offset = 0;
                    return;
                }
                KeyCode::Char('g') => {
                    self.log_auto_scroll = false;
                    self.log_scroll_offset = 0;
                    return;
                }
                _ => {} // Fall through to global keys
            }
        }

        // Normal mode key handling
        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Tab => {
                self.tab = self.tab.next();
                self.scroll_offset = 0;
            }
            KeyCode::BackTab => {
                self.tab = self.tab.prev();
                self.scroll_offset = 0;
            }
            KeyCode::Char(' ') => {
                if self.bp_paused {
                    // Resume from breakpoint pause
                    self.bp_paused = false;
                    self.bp_pause_reason = None;
                    self.paused = false;
                } else {
                    self.paused = !self.paused;
                }
            }
            KeyCode::Char('b') => {
                // Toggle breakpoint input dialog
                self.bp_input_mode = true;
                self.bp_input.clear();
            }
            KeyCode::Char('n') if self.bp_paused => {
                // Step one event while breakpoint-paused
                self.bp_step_one = true;
                self.bp_paused = false;
                self.paused = false;
            }
            KeyCode::Char('c') if !modifiers.contains(KeyModifiers::CONTROL) && self.bp_paused => {
                // Continue until next breakpoint
                self.bp_paused = false;
                self.bp_pause_reason = None;
                self.paused = false;
            }
            KeyCode::Char('/') => {
                self.filter_mode = true;
                self.filter_input.clear();
            }
            KeyCode::Esc => {
                self.filter = None;
                self.filter_input.clear();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                self.auto_scroll = false;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                if self.scroll_offset == 0 {
                    self.auto_scroll = true;
                }
            }
            KeyCode::Char('G') => {
                // Jump to bottom
                self.auto_scroll = true;
                self.scroll_offset = 0;
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.mem_graph_window_secs =
                    (self.mem_graph_window_secs + 10).min(300);
            }
            KeyCode::Char('-') => {
                self.mem_graph_window_secs =
                    self.mem_graph_window_secs.saturating_sub(10).max(10);
            }
            KeyCode::Char('C') => {
                // Load comparison from the two most recent sessions
                self.load_comparison_from_recent();
                self.tab = Tab::Compare;
                self.scroll_offset = 0;
            }
            _ => {}
        }
    }

    /// Cycle the log category filter through all categories (and None).
    fn cycle_log_category_filter(&mut self) {
        self.log_category_filter = match &self.log_category_filter {
            None => Some(LOG_CATEGORIES[0].to_string()),
            Some(current) => {
                let idx = LOG_CATEGORIES.iter().position(|&c| c == current);
                match idx {
                    Some(i) if i + 1 < LOG_CATEGORIES.len() => {
                        Some(LOG_CATEGORIES[i + 1].to_string())
                    }
                    _ => None, // Wrap around to None
                }
            }
        };
    }

    /// Cycle the log level filter through all levels (and None).
    fn cycle_log_level_filter(&mut self) {
        self.log_level_filter = match &self.log_level_filter {
            None => Some(LOG_LEVELS[0].to_string()),
            Some(current) => {
                let idx = LOG_LEVELS.iter().position(|&l| l == current);
                match idx {
                    Some(i) if i + 1 < LOG_LEVELS.len() => {
                        Some(LOG_LEVELS[i + 1].to_string())
                    }
                    _ => None, // Wrap around to None
                }
            }
        };
    }

    /// Get the numeric severity for a log level (higher = more severe).
    pub fn level_severity(level: &str) -> u8 {
        match level {
            "trace" => 0,
            "debug" => 1,
            "info" => 2,
            "warn" => 3,
            "error" => 4,
            "fatal" => 5,
            _ => 2, // Default to info
        }
    }

    /// Get log entries filtered by the current filters.
    pub fn filtered_log_entries(&self) -> Vec<&LogEntry> {
        self.log_entries
            .iter()
            .filter(|entry| {
                // Level filter: only show entries >= the filter level
                if let Some(ref min_level) = self.log_level_filter {
                    if Self::level_severity(&entry.level)
                        < Self::level_severity(min_level)
                    {
                        return false;
                    }
                }
                // Category filter: exact match
                if let Some(ref cat) = self.log_category_filter {
                    if entry.category != *cat {
                        return false;
                    }
                }
                // Source filter: substring match
                if let Some(ref src) = self.log_source_filter {
                    if !entry.source.contains(src.as_str()) {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    /// Handle a WebSocket event from the server.
    pub fn handle_ws_event(&mut self, event: WsEvent) {
        match event {
            WsEvent::ClientConnected {
                client_id,
                user_agent,
                device_memory,
            } => {
                self.clients.insert(
                    client_id.clone(),
                    ClientInfo {
                        client_id,
                        user_agent,
                        device_memory,
                        connected_at: Instant::now(),
                    },
                );
            }
            WsEvent::Trace(trace) => {
                if !self.paused {
                    // Check if this is a log entry (meta.level is non-empty)
                    let is_log_entry = trace
                        .meta
                        .as_ref()
                        .is_some_and(|m| !m.level.is_empty());

                    if is_log_entry {
                        self.ingest_log_entry(&trace);
                    }

                    // Check breakpoints before ingesting
                    let trigger = self.check_breakpoints(&trace);

                    self.ingest_event(trace);

                    // If a breakpoint triggered, pause
                    if let Some(reason) = trigger {
                        self.bp_paused = true;
                        self.bp_pause_reason = Some(reason);
                        self.bp_pause_index = Some(self.events.len().saturating_sub(1));
                        self.paused = true;
                    }

                    // If step mode was active, re-pause after one event
                    if self.bp_step_one {
                        self.bp_step_one = false;
                        self.bp_paused = true;
                        self.bp_pause_reason = Some("Step".to_string());
                        self.paused = true;
                    }
                }
            }
            WsEvent::ClientDisconnected { client_id, clean } => {
                if !clean {
                    self.generate_crash_report(&client_id);
                }
                self.clients.remove(&client_id);
            }
        }
    }

    /// Ingest a trace event into the app state.
    fn ingest_event(&mut self, event: TraceEvent) {
        // Update per-function stats
        let stats = self.func_stats.entry(event.func.clone()).or_default();
        stats.call_count += 1;
        stats.total_dur_ms += event.dur_ms;
        stats.total_mem_growth += event.mem_growth;
        stats.recent_calls.push(event.ts);

        // Write to session file
        if let Some(ref mut writer) = self.session_writer {
            let _ = writer.write_event(&event);
        }

        // Circular buffer: remove oldest if at capacity
        if self.events.len() >= MAX_EVENTS {
            self.events.remove(0);
        }

        self.events.push(event);
    }

    /// Check all breakpoints against a trace event.
    /// Returns the reason string if any breakpoint triggered.
    fn check_breakpoints(&self, event: &TraceEvent) -> Option<String> {
        for bp in &self.breakpoints {
            if let Some(reason) = bp.should_trigger(event, self) {
                return Some(reason);
            }
        }
        None
    }

    /// Ingest a trace event as a log entry into the Log tab.
    fn ingest_log_entry(&mut self, event: &TraceEvent) {
        let meta = match &event.meta {
            Some(m) => m,
            None => return,
        };

        let entry = LogEntry {
            timestamp: event.ts,
            level: meta.level.clone(),
            category: event.cat.clone(),
            source: meta.src.clone(),
            message: event.func.clone(),
            data: meta.data.clone(),
        };

        if self.log_entries.len() >= MAX_LOG_ENTRIES {
            self.log_entries.pop_front();
        }
        self.log_entries.push_back(entry);
    }

    /// Periodic tick — update computed metrics.
    pub fn tick(&mut self) {
        self.tick_count += 1;

        // Calculate events/sec (every tick = 200ms, so 5 ticks = 1s)
        let current_count = self.events.len();
        let delta = current_count.saturating_sub(self.events_at_last_tick);
        // Exponential moving average (alpha=0.3)
        let instant_rate = delta as f64 * 5.0; // 200ms tick → multiply by 5 for /sec
        self.events_per_sec = self.events_per_sec * 0.7 + instant_rate * 0.3;
        self.events_at_last_tick = current_count;

        // Trim old recent_calls data (keep last 10s window)
        if let Some(latest_ts) = self.events.last().map(|e| e.ts) {
            let cutoff = latest_ts - 10_000.0;
            for stats in self.func_stats.values_mut() {
                stats.recent_calls.retain(|&ts| ts > cutoff);
            }
        }
    }

    /// Generate a crash report when a client disconnects unexpectedly.
    fn generate_crash_report(&self, client_id: &str) {
        if let Err(e) = crate::crash_report::generate(self, client_id) {
            eprintln!("Failed to generate crash report: {e}");
        }
    }

    /// Load a comparison from the two most recent session files.
    pub fn load_comparison_from_recent(&mut self) {
        let log_dir = crate::store::log_dir();
        match crate::store::list_sessions(&log_dir) {
            Ok(sessions) if sessions.len() >= 2 => {
                match crate::store::compare::compare_sessions(
                    &sessions[1], // older = A
                    &sessions[0], // newer = B
                ) {
                    Ok(comp) => self.comparison = Some(comp),
                    Err(e) => eprintln!("Comparison failed: {e}"),
                }
            }
            Ok(_) => {
                eprintln!("Need at least 2 sessions for comparison");
            }
            Err(e) => eprintln!("Failed to list sessions: {e}"),
        }
    }

    /// Load a comparison from two specific session file paths.
    pub fn load_comparison_from_files(
        &mut self,
        path_a: &std::path::Path,
        path_b: &std::path::Path,
    ) {
        match crate::store::compare::compare_sessions(path_a, path_b) {
            Ok(comp) => self.comparison = Some(comp),
            Err(e) => eprintln!("Comparison failed: {e}"),
        }
    }

    /// Load a replay session from the latest saved session file.
    pub fn load_replay_from_latest(&mut self) {
        let log_dir = crate::store::log_dir();
        match crate::store::list_sessions(&log_dir) {
            Ok(sessions) if !sessions.is_empty() => {
                let path = sessions[0].clone();
                self.load_replay_from_file(&path);
            }
            Ok(_) => {
                eprintln!("No session files found for replay");
            }
            Err(e) => eprintln!("Failed to list sessions: {e}"),
        }
    }

    /// Load a replay session from a specific file path.
    pub fn load_replay_from_file(&mut self, path: &std::path::Path) {
        match ReplaySession::load(path) {
            Ok(session) => {
                self.replay_session = Some(session);
                self.tab = Tab::Replay;
            }
            Err(e) => eprintln!("Failed to load replay session: {e}"),
        }
    }

    /// Build or rebuild the dependency graph from current events.
    pub fn build_deps_graph(&mut self) {
        let mut state = DepsState::new();
        state.build_from_events(&self.events);
        self.deps_state = Some(state);
    }

    /// Render the full UI.
    pub fn render(&self, frame: &mut Frame) {
        ui::render(frame, self);
    }

    /// Get the latest memory value across all events.
    pub fn latest_mem(&self) -> i64 {
        self.events
            .iter()
            .rev()
            .find(|e| e.mem_after > 0)
            .map(|e| e.mem_after)
            .unwrap_or(0)
    }

    /// Get total memory growth this session.
    pub fn total_mem_growth(&self) -> i64 {
        self.func_stats.values().map(|s| s.total_mem_growth).sum()
    }

    /// Get the session duration as a formatted string.
    pub fn session_duration(&self) -> String {
        let secs = self.session_start.elapsed().as_secs();
        let mins = secs / 60;
        let secs = secs % 60;
        format!("{mins}m{secs:02}s")
    }

    /// Get events filtered by the current regex filter.
    pub fn filtered_events(&self) -> Vec<&TraceEvent> {
        match &self.filter {
            Some(re) => self
                .events
                .iter()
                .filter(|e| {
                    re.is_match(&e.func)
                        || re.is_match(&e.cat)
                        || e.err.as_deref().is_some_and(|err| re.is_match(err))
                })
                .collect(),
            None => self.events.iter().collect(),
        }
    }

    /// Get events filtered by category.
    pub fn events_by_cat(&self, cat: &str) -> Vec<&TraceEvent> {
        self.events.iter().filter(|e| e.cat == cat).collect()
    }
}
