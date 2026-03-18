//! Session comparison logic.
//!
//! Loads two JSONL sessions and produces a `SessionComparison`
//! containing side-by-side metrics and diffs for the Compare tab.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use color_eyre::eyre::Result;

use crate::app::TraceEvent;
use crate::store;

/// Summary statistics for a single session.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub path: PathBuf,
    pub duration_secs: f64,
    pub total_events: usize,
    pub total_errors: usize,
    pub max_memory: i64,
    pub avg_render_rate: f64,
}

/// Side-by-side comparison of two debug sessions.
#[derive(Debug, Clone)]
pub struct SessionComparison {
    pub session_a: SessionSummary,
    pub session_b: SessionSummary,
    /// (timestamp_offset_sec, a_mem, b_mem) for memory overlay graph.
    pub memory_diff: Vec<(f64, i64, i64)>,
    /// component -> (a_rate, b_rate) renders per second.
    pub render_rate_diff: HashMap<String, (f64, f64)>,
    /// category -> (a_count, b_count) event frequencies.
    pub event_freq_diff: HashMap<String, (u64, u64)>,
    /// Error messages in session B that were not in session A.
    pub new_errors: Vec<String>,
    /// (func, a_avg_ms, b_avg_ms) for WASM timing regressions.
    pub wasm_regressions: Vec<(String, f64, f64)>,
}

/// Build a `SessionSummary` from a vec of trace events.
fn summarise(path: &Path, events: &[TraceEvent]) -> SessionSummary {
    let total_events = events.len();

    let total_errors = events
        .iter()
        .filter(|e| e.err.is_some() || e.cat == "error")
        .count();

    let max_memory = events.iter().map(|e| e.mem_after).max().unwrap_or(0);

    // Duration: difference between first and last timestamp (ms -> sec).
    let duration_secs = if events.len() >= 2 {
        let first = events.first().map(|e| e.ts).unwrap_or(0.0);
        let last = events.last().map(|e| e.ts).unwrap_or(0.0);
        (last - first) / 1000.0
    } else {
        0.0
    };

    // Average render rate: count render events per second.
    let render_count = events.iter().filter(|e| e.cat == "render").count();
    let avg_render_rate = if duration_secs > 0.0 {
        render_count as f64 / duration_secs
    } else {
        0.0
    };

    SessionSummary {
        path: path.to_path_buf(),
        duration_secs,
        total_events,
        total_errors,
        max_memory,
        avg_render_rate,
    }
}

/// Compare two session files and produce a `SessionComparison`.
pub fn compare_sessions(
    path_a: &Path,
    path_b: &Path,
) -> Result<SessionComparison> {
    let events_a = store::load_session(path_a)?;
    let events_b = store::load_session(path_b)?;

    let session_a = summarise(path_a, &events_a);
    let session_b = summarise(path_b, &events_b);

    let memory_diff = build_memory_diff(&events_a, &events_b);
    let render_rate_diff = build_render_rate_diff(&events_a, &events_b);
    let event_freq_diff = build_event_freq_diff(&events_a, &events_b);
    let new_errors = find_new_errors(&events_a, &events_b);
    let wasm_regressions = find_wasm_regressions(&events_a, &events_b);

    Ok(SessionComparison {
        session_a,
        session_b,
        memory_diff,
        render_rate_diff,
        event_freq_diff,
        new_errors,
        wasm_regressions,
    })
}

/// Build a normalised memory timeline with values from both sessions.
/// We sample at ~100 points across the longer session's duration.
fn build_memory_diff(
    events_a: &[TraceEvent],
    events_b: &[TraceEvent],
) -> Vec<(f64, i64, i64)> {
    let ts_a: Vec<(f64, i64)> = events_a
        .iter()
        .filter(|e| e.mem_after > 0)
        .map(|e| (e.ts, e.mem_after))
        .collect();
    let ts_b: Vec<(f64, i64)> = events_b
        .iter()
        .filter(|e| e.mem_after > 0)
        .map(|e| (e.ts, e.mem_after))
        .collect();

    if ts_a.is_empty() && ts_b.is_empty() {
        return Vec::new();
    }

    // Normalise: convert timestamps to seconds from session start.
    let start_a = ts_a.first().map(|(t, _)| *t).unwrap_or(0.0);
    let start_b = ts_b.first().map(|(t, _)| *t).unwrap_or(0.0);

    let end_a = ts_a.last().map(|(t, _)| *t).unwrap_or(0.0);
    let end_b = ts_b.last().map(|(t, _)| *t).unwrap_or(0.0);

    let dur_a = (end_a - start_a) / 1000.0;
    let dur_b = (end_b - start_b) / 1000.0;
    let max_dur = dur_a.max(dur_b).max(0.001);

    let num_points = 100usize;
    let step = max_dur / num_points as f64;

    let mut result = Vec::with_capacity(num_points);
    for i in 0..num_points {
        let t = i as f64 * step;
        let mem_a = sample_mem_at(&ts_a, start_a, t);
        let mem_b = sample_mem_at(&ts_b, start_b, t);
        result.push((t, mem_a, mem_b));
    }

    result
}

/// Find the memory value at a given offset (seconds) from session start.
fn sample_mem_at(points: &[(f64, i64)], start: f64, offset_secs: f64) -> i64 {
    let target_ts = start + offset_secs * 1000.0;
    // Find the last point <= target_ts.
    let mut last_val = 0i64;
    for (ts, mem) in points {
        if *ts <= target_ts {
            last_val = *mem;
        } else {
            break;
        }
    }
    last_val
}

/// Per-component render rate comparison.
fn build_render_rate_diff(
    events_a: &[TraceEvent],
    events_b: &[TraceEvent],
) -> HashMap<String, (f64, f64)> {
    let rate_a = render_rates(events_a);
    let rate_b = render_rates(events_b);

    let mut all_keys: HashSet<String> = rate_a.keys().cloned().collect();
    all_keys.extend(rate_b.keys().cloned());

    let mut result = HashMap::new();
    for key in all_keys {
        let a = rate_a.get(&key).copied().unwrap_or(0.0);
        let b = rate_b.get(&key).copied().unwrap_or(0.0);
        result.insert(key, (a, b));
    }
    result
}

/// Calculate per-function render rate (renders/sec) for render events.
fn render_rates(events: &[TraceEvent]) -> HashMap<String, f64> {
    let renders: Vec<&TraceEvent> = events
        .iter()
        .filter(|e| e.cat == "render")
        .collect();

    if renders.is_empty() {
        return HashMap::new();
    }

    let first_ts = renders.first().map(|e| e.ts).unwrap_or(0.0);
    let last_ts = renders.last().map(|e| e.ts).unwrap_or(0.0);
    let dur_secs = ((last_ts - first_ts) / 1000.0).max(0.001);

    let mut counts: HashMap<String, usize> = HashMap::new();
    for ev in &renders {
        *counts.entry(ev.func.clone()).or_default() += 1;
    }

    counts
        .into_iter()
        .map(|(k, v)| (k, v as f64 / dur_secs))
        .collect()
}

/// Event frequency by category comparison.
fn build_event_freq_diff(
    events_a: &[TraceEvent],
    events_b: &[TraceEvent],
) -> HashMap<String, (u64, u64)> {
    let freq_a = event_freqs(events_a);
    let freq_b = event_freqs(events_b);

    let mut all_keys: HashSet<String> = freq_a.keys().cloned().collect();
    all_keys.extend(freq_b.keys().cloned());

    let mut result = HashMap::new();
    for key in all_keys {
        let a = freq_a.get(&key).copied().unwrap_or(0);
        let b = freq_b.get(&key).copied().unwrap_or(0);
        result.insert(key, (a, b));
    }
    result
}

/// Count events per category.
fn event_freqs(events: &[TraceEvent]) -> HashMap<String, u64> {
    let mut counts: HashMap<String, u64> = HashMap::new();
    for ev in events {
        *counts.entry(ev.cat.clone()).or_default() += 1;
    }
    counts
}

/// Find error messages in session B not present in session A.
fn find_new_errors(
    events_a: &[TraceEvent],
    events_b: &[TraceEvent],
) -> Vec<String> {
    let errors_a: HashSet<String> = events_a
        .iter()
        .filter_map(|e| e.err.clone())
        .collect();

    let mut new: Vec<String> = events_b
        .iter()
        .filter_map(|e| e.err.clone())
        .filter(|msg| !errors_a.contains(msg))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    new.sort();
    new
}

/// Find WASM functions where avg duration increased (regressions).
/// Only includes functions present in both sessions.
fn find_wasm_regressions(
    events_a: &[TraceEvent],
    events_b: &[TraceEvent],
) -> Vec<(String, f64, f64)> {
    let avg_a = wasm_avg_durations(events_a);
    let avg_b = wasm_avg_durations(events_b);

    let mut regressions: Vec<(String, f64, f64)> = avg_b
        .iter()
        .filter_map(|(func, &b_avg)| {
            avg_a.get(func).map(|&a_avg| {
                (func.clone(), a_avg, b_avg)
            })
        })
        .filter(|(_, a, b)| *b > *a * 1.1) // At least 10% slower
        .collect();

    regressions.sort_by(|a, b| {
        (b.2 - b.1)
            .partial_cmp(&(a.2 - a.1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    regressions
}

/// Average duration per WASM function.
fn wasm_avg_durations(events: &[TraceEvent]) -> HashMap<String, f64> {
    let mut totals: HashMap<String, (f64, usize)> = HashMap::new();
    for ev in events.iter().filter(|e| e.cat == "wasm" && e.dur_ms > 0.0) {
        let entry = totals.entry(ev.func.clone()).or_default();
        entry.0 += ev.dur_ms;
        entry.1 += 1;
    }
    totals
        .into_iter()
        .map(|(k, (total, count))| (k, total / count as f64))
        .collect()
}
