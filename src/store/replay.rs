//! Interactive replay session with time-travel debugging.
//!
//! Loads a saved JSONL session and provides cursor-based navigation
//! with forward/backward stepping, bookmarks, speed control, and
//! reconstructed state at any point in the event timeline.

use std::collections::HashMap;
use std::path::Path;

use color_eyre::eyre::Result;

use crate::app::TraceEvent;
use crate::store;

/// Replay speed control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaySpeed {
    Paused,
    X1,
    X2,
    X5,
    X10,
}

impl ReplaySpeed {
    pub fn label(self) -> &'static str {
        match self {
            ReplaySpeed::Paused => "Paused",
            ReplaySpeed::X1 => "1x",
            ReplaySpeed::X2 => "2x",
            ReplaySpeed::X5 => "5x",
            ReplaySpeed::X10 => "10x",
        }
    }

    /// Milliseconds between auto-advance steps (None = paused).
    pub fn step_interval_ms(self) -> Option<u64> {
        match self {
            ReplaySpeed::Paused => None,
            ReplaySpeed::X1 => Some(200),
            ReplaySpeed::X2 => Some(100),
            ReplaySpeed::X5 => Some(40),
            ReplaySpeed::X10 => Some(20),
        }
    }
}

/// Reconstructed state at the current cursor position.
#[derive(Debug, Clone, Default)]
pub struct ReplayState {
    /// Render counts per component at cursor.
    pub render_counts: HashMap<String, u64>,
    /// Memory timeline up to cursor: (timestamp, mem_after).
    pub memory_timeline: Vec<(f64, i64)>,
    /// Errors encountered up to cursor.
    pub error_list: Vec<String>,
    /// Total events processed up to cursor.
    pub events_processed: usize,
    /// Total memory growth up to cursor.
    pub total_mem_growth: i64,
}

/// Interactive replay session with cursor-based navigation.
pub struct ReplaySession {
    /// All events loaded from the session file.
    events: Vec<TraceEvent>,
    /// Current cursor position (0-indexed).
    cursor: usize,
    /// Playback speed.
    pub speed: ReplaySpeed,
    /// Bookmarked cursor positions.
    bookmarks: Vec<usize>,
    /// Text filter for event search.
    filter: Option<String>,
    /// Whether the filter input mode is active.
    pub filter_mode: bool,
    /// Current filter input text being typed.
    pub filter_input: String,
    /// Whether an event detail view is expanded.
    pub detail_expanded: bool,
    /// Scroll offset for the event list.
    pub scroll_offset: usize,
    /// Cached reconstructed state (rebuilt on cursor move).
    cached_state: ReplayState,
    /// Session file path for display.
    pub session_path: String,
}

impl ReplaySession {
    /// Load a JSONL session file into a replay session.
    pub fn load(path: &Path) -> Result<Self> {
        let events = store::load_session(path)?;
        let session_path = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let mut session = Self {
            events,
            cursor: 0,
            speed: ReplaySpeed::Paused,
            bookmarks: Vec::new(),
            filter: None,
            filter_mode: false,
            filter_input: String::new(),
            detail_expanded: false,
            scroll_offset: 0,
            cached_state: ReplayState::default(),
            session_path,
        };

        session.rebuild_state();
        Ok(session)
    }

    /// Total number of events in the session.
    pub fn total_events(&self) -> usize {
        self.events.len()
    }

    /// Current cursor position.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Step the cursor forward by one event.
    pub fn step_forward(&mut self) {
        if self.cursor < self.events.len().saturating_sub(1) {
            self.cursor += 1;
            self.rebuild_state();
        }
    }

    /// Step the cursor backward by one event.
    pub fn step_backward(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.rebuild_state();
        }
    }

    /// Jump to a specific cursor position.
    pub fn goto(&mut self, pos: usize) {
        let max = self.events.len().saturating_sub(1);
        self.cursor = pos.min(max);
        self.rebuild_state();
    }

    /// Jump to the start of the session.
    pub fn goto_start(&mut self) {
        self.cursor = 0;
        self.rebuild_state();
    }

    /// Jump to the end of the session.
    pub fn goto_end(&mut self) {
        self.cursor = self.events.len().saturating_sub(1);
        self.rebuild_state();
    }

    /// Set the replay speed.
    pub fn set_speed(&mut self, speed: ReplaySpeed) {
        self.speed = speed;
    }

    /// Toggle bookmark at the current cursor position.
    pub fn toggle_bookmark(&mut self) {
        if let Some(idx) = self.bookmarks.iter().position(|&b| b == self.cursor) {
            self.bookmarks.remove(idx);
        } else {
            self.bookmarks.push(self.cursor);
            self.bookmarks.sort_unstable();
        }
    }

    /// Get all bookmark positions.
    pub fn bookmarks(&self) -> &[usize] {
        &self.bookmarks
    }

    /// Jump to the next bookmark after the current cursor.
    pub fn goto_next_bookmark(&mut self) {
        if let Some(&pos) = self.bookmarks.iter().find(|&&b| b > self.cursor) {
            self.cursor = pos;
            self.rebuild_state();
        }
    }

    /// Jump to the previous bookmark before the current cursor.
    pub fn goto_prev_bookmark(&mut self) {
        if let Some(&pos) = self.bookmarks.iter().rev().find(|&&b| b < self.cursor) {
            self.cursor = pos;
            self.rebuild_state();
        }
    }

    /// Set the text filter for event search.
    pub fn set_filter(&mut self, filter: Option<String>) {
        self.filter = filter;
    }

    /// Get the current text filter.
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    /// Get the event at the current cursor position.
    pub fn current_event(&self) -> Option<&TraceEvent> {
        self.events.get(self.cursor)
    }

    /// Get a window of events around the cursor for display.
    pub fn visible_events(
        &self,
        window_size: usize,
    ) -> (Vec<(usize, &TraceEvent)>, usize) {
        let total = self.events.len();
        if total == 0 {
            return (Vec::new(), 0);
        }

        let half = window_size / 2;
        let start = self.cursor.saturating_sub(half);
        let end = (start + window_size).min(total);
        let start = if end == total {
            total.saturating_sub(window_size)
        } else {
            start
        };

        let events: Vec<(usize, &TraceEvent)> = self.events[start..end]
            .iter()
            .enumerate()
            .map(|(i, ev)| (start + i, ev))
            .collect();

        let cursor_in_window = self.cursor - start;
        (events, cursor_in_window)
    }

    /// Get filtered events (matching the text filter).
    pub fn filtered_event_indices(&self) -> Vec<usize> {
        match &self.filter {
            Some(f) => {
                let f_lower = f.to_lowercase();
                self.events
                    .iter()
                    .enumerate()
                    .filter(|(_, ev)| {
                        ev.func.to_lowercase().contains(&f_lower)
                            || ev.cat.to_lowercase().contains(&f_lower)
                            || ev
                                .err
                                .as_deref()
                                .is_some_and(|e| e.to_lowercase().contains(&f_lower))
                    })
                    .map(|(i, _)| i)
                    .collect()
            }
            None => (0..self.events.len()).collect(),
        }
    }

    /// Jump to the next event matching the filter.
    pub fn goto_next_match(&mut self) {
        let indices = self.filtered_event_indices();
        if let Some(&pos) = indices.iter().find(|&&i| i > self.cursor) {
            self.cursor = pos;
            self.rebuild_state();
        }
    }

    /// Jump to the previous event matching the filter.
    pub fn goto_prev_match(&mut self) {
        let indices = self.filtered_event_indices();
        if let Some(&pos) = indices.iter().rev().find(|&&i| i < self.cursor) {
            self.cursor = pos;
            self.rebuild_state();
        }
    }

    /// Get the reconstructed state at the current cursor.
    pub fn state_at_cursor(&self) -> &ReplayState {
        &self.cached_state
    }

    /// Check whether the cursor is at a bookmarked position.
    pub fn is_bookmarked(&self, pos: usize) -> bool {
        self.bookmarks.contains(&pos)
    }

    /// Rebuild the reconstructed state up to the current cursor.
    fn rebuild_state(&mut self) {
        let mut state = ReplayState::default();
        let end = (self.cursor + 1).min(self.events.len());

        for ev in &self.events[..end] {
            // Track render counts (events with "render" category)
            if ev.cat == "wasm" || ev.cat == "render" {
                *state.render_counts.entry(ev.func.clone()).or_default() += 1;
            }

            // Track memory timeline
            if ev.mem_after > 0 {
                state.memory_timeline.push((ev.ts, ev.mem_after));
            }

            // Collect errors
            if let Some(ref err) = ev.err {
                state
                    .error_list
                    .push(format!("[{}] {}: {err}", ev.cat, ev.func));
            }

            // Accumulate memory growth
            state.total_mem_growth += ev.mem_growth;
        }

        state.events_processed = end;
        self.cached_state = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_event(seq: u64, func: &str, cat: &str) -> TraceEvent {
        TraceEvent {
            seq,
            ts: seq as f64 * 100.0,
            cat: cat.to_string(),
            func: func.to_string(),
            arg_bytes: 0,
            arg_preview: None,
            dur_ms: 1.0,
            mem_before: 1000,
            mem_after: 1000 + (seq as i64 * 100),
            mem_growth: seq as i64 * 10,
            sql_context: None,
            client_id: "test".to_string(),
            err: if seq == 3 {
                Some("test error".to_string())
            } else {
                None
            },
            meta: None,
        }
    }

    fn write_session_file(events: &[TraceEvent]) -> NamedTempFile {
        let mut file = NamedTempFile::with_suffix(".jsonl").unwrap();
        for ev in events {
            let json = serde_json::to_string(ev).unwrap();
            writeln!(file, "{json}").unwrap();
        }
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_load_and_navigate() {
        let events: Vec<TraceEvent> = (0..5)
            .map(|i| make_event(i, &format!("func_{i}"), "wasm"))
            .collect();
        let file = write_session_file(&events);

        let mut session = ReplaySession::load(file.path()).unwrap();
        assert_eq!(session.total_events(), 5);
        assert_eq!(session.cursor(), 0);

        session.step_forward();
        assert_eq!(session.cursor(), 1);

        session.step_forward();
        session.step_forward();
        assert_eq!(session.cursor(), 3);

        session.step_backward();
        assert_eq!(session.cursor(), 2);

        session.goto_end();
        assert_eq!(session.cursor(), 4);

        session.goto_start();
        assert_eq!(session.cursor(), 0);
    }

    #[test]
    fn test_bookmarks() {
        let events: Vec<TraceEvent> = (0..10)
            .map(|i| make_event(i, &format!("func_{i}"), "wasm"))
            .collect();
        let file = write_session_file(&events);

        let mut session = ReplaySession::load(file.path()).unwrap();

        // Bookmark at position 0
        session.toggle_bookmark();
        assert_eq!(session.bookmarks(), &[0]);

        // Move to position 5 and bookmark
        session.goto(5);
        session.toggle_bookmark();
        assert_eq!(session.bookmarks(), &[0, 5]);

        // Navigate between bookmarks
        session.goto_start();
        session.goto_next_bookmark();
        assert_eq!(session.cursor(), 5);

        session.goto_prev_bookmark();
        assert_eq!(session.cursor(), 0);

        // Remove bookmark
        session.toggle_bookmark();
        assert_eq!(session.bookmarks(), &[5]);
    }

    #[test]
    fn test_state_reconstruction() {
        let events: Vec<TraceEvent> = (0..5)
            .map(|i| make_event(i, &format!("func_{i}"), "wasm"))
            .collect();
        let file = write_session_file(&events);

        let mut session = ReplaySession::load(file.path()).unwrap();

        // At cursor 0, we have 1 event processed
        let state = session.state_at_cursor();
        assert_eq!(state.events_processed, 1);

        // At cursor 4 (end), all 5 processed
        session.goto_end();
        let state = session.state_at_cursor();
        assert_eq!(state.events_processed, 5);
        // Event 3 had an error
        assert_eq!(state.error_list.len(), 1);
    }

    #[test]
    fn test_filter() {
        let events = vec![
            make_event(0, "init_module", "wasm"),
            make_event(1, "render_view", "render"),
            make_event(2, "sql_query", "sql"),
            make_event(3, "render_button", "render"),
            make_event(4, "net_fetch", "net"),
        ];
        let file = write_session_file(&events);

        let mut session = ReplaySession::load(file.path()).unwrap();
        session.set_filter(Some("render".to_string()));

        let indices = session.filtered_event_indices();
        assert_eq!(indices, vec![1, 3]);

        // Navigate to matches
        session.goto_next_match();
        assert_eq!(session.cursor(), 1);

        session.goto_next_match();
        assert_eq!(session.cursor(), 3);
    }

    #[test]
    fn test_speed_labels() {
        assert_eq!(ReplaySpeed::Paused.label(), "Paused");
        assert_eq!(ReplaySpeed::X1.label(), "1x");
        assert_eq!(ReplaySpeed::X10.label(), "10x");
        assert!(ReplaySpeed::Paused.step_interval_ms().is_none());
        assert!(ReplaySpeed::X1.step_interval_ms().is_some());
    }
}
