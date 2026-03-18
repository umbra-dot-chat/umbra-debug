//! CLI query functions for inspecting saved sessions.
//!
//! These functions are used by the `query` subcommand to print
//! analysis results to stdout without launching the TUI.

use std::collections::HashMap;
use std::path::Path;

use color_eyre::eyre::{eyre, Result};
use regex::Regex;

use crate::app::TraceEvent;
use crate::store;

/// Print the contents of the last crash report.
pub fn print_last_crash(log_dir: &Path) -> Result<()> {
    match store::find_latest_crash(log_dir)? {
        Some(path) => {
            let content = std::fs::read_to_string(&path)?;
            println!("{content}");
        }
        None => {
            println!("No crash reports found.");
        }
    }
    Ok(())
}

/// Print top 20 functions by total memory growth.
pub fn print_memory_suspects(log_dir: &Path) -> Result<()> {
    let events = load_latest_session(log_dir)?;
    let mut growth_map: HashMap<String, i64> = HashMap::new();

    for ev in &events {
        if ev.mem_growth != 0 {
            *growth_map.entry(ev.func.clone()).or_default() += ev.mem_growth;
        }
    }

    let mut suspects: Vec<(String, i64)> = growth_map.into_iter().collect();
    suspects.sort_by(|a, b| b.1.abs().cmp(&a.1.abs()));

    println!("{:<50} {:>15}", "Function", "Total Growth");
    println!("{}", "-".repeat(67));
    for (func, growth) in suspects.iter().take(20) {
        println!("{:<50} {:>15}", func, format_bytes(*growth));
    }
    Ok(())
}

/// Print top 20 functions by call count.
pub fn print_hot_functions(log_dir: &Path) -> Result<()> {
    let events = load_latest_session(log_dir)?;
    let mut count_map: HashMap<String, usize> = HashMap::new();

    for ev in &events {
        *count_map.entry(ev.func.clone()).or_default() += 1;
    }

    let mut hot: Vec<(String, usize)> = count_map.into_iter().collect();
    hot.sort_by(|a, b| b.1.cmp(&a.1));

    println!("{:<50} {:>10}", "Function", "Calls");
    println!("{}", "-".repeat(62));
    for (func, count) in hot.iter().take(20) {
        println!("{:<50} {:>10}", func, count);
    }
    Ok(())
}

/// Regex search across all session files.
pub fn print_grep(log_dir: &Path, pattern: &str) -> Result<()> {
    let re = Regex::new(pattern)?;
    let sessions = store::list_sessions(log_dir)?;

    for session_path in sessions {
        let events = store::load_session(&session_path)?;
        let filename = session_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();

        for ev in &events {
            let line = serde_json::to_string(ev)?;
            if re.is_match(&line) {
                println!("[{filename}] seq={} {} {}", ev.seq, ev.cat, ev.func);
            }
        }
    }
    Ok(())
}

/// Print ASCII memory timeline from the last session.
pub fn print_memory_timeline(log_dir: &Path) -> Result<()> {
    let events = load_latest_session(log_dir)?;

    if events.is_empty() {
        println!("No events in last session.");
        return Ok(());
    }

    // Collect memory snapshots over time
    let mut mem_points: Vec<(f64, i64)> = Vec::new();
    for ev in &events {
        if ev.mem_after > 0 {
            mem_points.push((ev.ts, ev.mem_after));
        }
    }

    if mem_points.is_empty() {
        println!("No memory data in last session.");
        return Ok(());
    }

    // Print a simple ASCII sparkline of memory over time
    let max_mem = mem_points.iter().map(|(_, m)| *m).max().unwrap_or(1);
    let min_mem = mem_points.iter().map(|(_, m)| *m).min().unwrap_or(0);
    let range = (max_mem - min_mem).max(1);

    let width = 60;
    let step = mem_points.len().max(1) / width.max(1);
    let step = step.max(1);

    println!("Memory Timeline (last session)");
    println!("Max: {}  Min: {}", format_bytes(max_mem), format_bytes(min_mem));
    println!();

    let bars = [' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}'];
    let mut sparkline = String::new();

    for chunk in mem_points.chunks(step) {
        let avg = chunk.iter().map(|(_, m)| *m).sum::<i64>() / chunk.len() as i64;
        let normalized = ((avg - min_mem) * 8 / range).min(8) as usize;
        sparkline.push(bars[normalized]);
    }

    println!("{sparkline}");
    Ok(())
}

/// Print WASM calls exceeding a duration threshold.
pub fn print_slow_wasm(log_dir: &Path, threshold_ms: f64) -> Result<()> {
    let events = load_latest_session(log_dir)?;

    let mut slow: Vec<&TraceEvent> = events
        .iter()
        .filter(|e| e.cat == "wasm" && e.dur_ms >= threshold_ms)
        .collect();

    slow.sort_by(|a, b| {
        b.dur_ms
            .partial_cmp(&a.dur_ms)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if slow.is_empty() {
        println!("No WASM calls exceeded {threshold_ms}ms.");
        return Ok(());
    }

    println!(
        "{:<50} {:>10} {:>15}",
        "Function", "Duration", "Timestamp"
    );
    println!("{}", "-".repeat(77));
    for ev in &slow {
        println!(
            "{:<50} {:>9.1}ms {:>15.1}",
            ev.func, ev.dur_ms, ev.ts
        );
    }
    println!("\n{} calls exceeded {}ms threshold.", slow.len(), threshold_ms);
    Ok(())
}

/// Print components with render storms (>100 renders/sec).
pub fn print_render_storms(log_dir: &Path) -> Result<()> {
    let events = load_latest_session(log_dir)?;

    // Find events flagged as render storms
    let storms: Vec<&TraceEvent> = events
        .iter()
        .filter(|e| e.func.contains("RENDER STORM"))
        .collect();

    if storms.is_empty() {
        // Fallback: detect high-frequency renders manually
        let mut render_counts: HashMap<String, Vec<f64>> = HashMap::new();
        for ev in events.iter().filter(|e| e.cat == "render") {
            render_counts
                .entry(ev.func.clone())
                .or_default()
                .push(ev.ts);
        }

        let mut storm_components: Vec<(String, f64)> = Vec::new();
        for (name, timestamps) in &render_counts {
            if timestamps.len() < 2 {
                continue;
            }
            let first = timestamps.first().copied().unwrap_or(0.0);
            let last = timestamps.last().copied().unwrap_or(0.0);
            let dur_secs = (last - first) / 1000.0;
            if dur_secs > 0.0 {
                let rate = timestamps.len() as f64 / dur_secs;
                if rate > 100.0 {
                    storm_components.push((name.clone(), rate));
                }
            }
        }

        if storm_components.is_empty() {
            println!("No render storms detected.");
            return Ok(());
        }

        storm_components
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        println!("{:<50} {:>12}", "Component", "Peak Rate");
        println!("{}", "-".repeat(64));
        for (name, rate) in &storm_components {
            println!("{:<50} {:>10.0}/s", name, rate);
        }
        return Ok(());
    }

    println!("{:<50} {:>15}", "Storm Event", "Timestamp");
    println!("{}", "-".repeat(67));
    for ev in &storms {
        println!("{:<50} {:>15.1}", ev.func, ev.ts);
    }
    Ok(())
}

/// Print state change events matching a context name.
pub fn print_state_changes(log_dir: &Path, context: &str) -> Result<()> {
    let events = load_latest_session(log_dir)?;

    let matches: Vec<&TraceEvent> = events
        .iter()
        .filter(|e| {
            e.cat == "state"
                && (e.func.contains(context)
                    || e.arg_preview
                        .as_deref()
                        .is_some_and(|p| p.contains(context)))
        })
        .collect();

    if matches.is_empty() {
        println!("No state changes found for '{context}'.");
        return Ok(());
    }

    println!("{:<15} {:<40} Preview", "Timestamp", "Function");
    println!("{}", "-".repeat(80));
    for ev in &matches {
        let preview = ev.arg_preview.as_deref().unwrap_or("-");
        let truncated = if preview.len() > 40 {
            format!("{}...", &preview[..37])
        } else {
            preview.to_string()
        };
        println!("{:<15.1} {:<40} {}", ev.ts, ev.func, truncated);
    }
    println!("\n{} state changes for '{context}'.", matches.len());
    Ok(())
}

/// Print events within a time range (HH:MM format).
pub fn print_timeline(log_dir: &Path, start: &str, end: &str) -> Result<()> {
    let events = load_latest_session(log_dir)?;

    if events.is_empty() {
        println!("No events in last session.");
        return Ok(());
    }

    let start_offset = parse_hhmm_to_ms(start)?;
    let end_offset = parse_hhmm_to_ms(end)?;

    // Rebase timestamps relative to session start
    let session_start = events.first().map(|e| e.ts).unwrap_or(0.0);

    let filtered: Vec<&TraceEvent> = events
        .iter()
        .filter(|e| {
            let offset = e.ts - session_start;
            offset >= start_offset && offset <= end_offset
        })
        .collect();

    if filtered.is_empty() {
        println!("No events between {start} and {end}.");
        return Ok(());
    }

    println!(
        "{:<10} {:<8} {:<40} {:>10}",
        "Offset", "Cat", "Function", "Duration"
    );
    println!("{}", "-".repeat(72));
    for ev in &filtered {
        let offset_ms = ev.ts - session_start;
        let secs = offset_ms / 1000.0;
        let mins = (secs / 60.0).floor();
        let secs_rem = secs % 60.0;
        println!(
            "{:>3}:{:05.2} {:<8} {:<40} {:>9.1}ms",
            mins, secs_rem, ev.cat, ev.func, ev.dur_ms
        );
    }
    println!("\n{} events in range.", filtered.len());
    Ok(())
}

/// Print only error and fatal level events.
pub fn print_errors_only(log_dir: &Path) -> Result<()> {
    let events = load_latest_session(log_dir)?;

    let errors: Vec<&TraceEvent> = events
        .iter()
        .filter(|e| {
            e.err.is_some()
                || e.cat == "error"
                || e.meta
                    .as_ref()
                    .is_some_and(|m| m.level == "error" || m.level == "fatal")
        })
        .collect();

    if errors.is_empty() {
        println!("No errors found in last session.");
        return Ok(());
    }

    println!("{:<15} {:<10} Message", "Timestamp", "Category");
    println!("{}", "-".repeat(80));
    for ev in &errors {
        let msg = ev
            .err
            .as_deref()
            .unwrap_or(&ev.func);
        let truncated = if msg.len() > 55 {
            format!("{}...", &msg[..52])
        } else {
            msg.to_string()
        };
        println!("{:<15.1} {:<10} {}", ev.ts, ev.cat, truncated);
    }
    println!("\n{} error events.", errors.len());
    Ok(())
}

/// Print budget violation events.
pub fn print_budget_violations(log_dir: &Path) -> Result<()> {
    let events = load_latest_session(log_dir)?;

    let violations: Vec<&TraceEvent> = events
        .iter()
        .filter(|e| {
            e.func.contains("BUDGET EXCEEDED")
                || e.arg_preview
                    .as_deref()
                    .is_some_and(|p| p.contains("BUDGET EXCEEDED"))
        })
        .collect();

    if violations.is_empty() {
        println!("No budget violations found in last session.");
        return Ok(());
    }

    println!("{:<15} Violation", "Timestamp");
    println!("{}", "-".repeat(80));
    for ev in &violations {
        let detail = ev
            .arg_preview
            .as_deref()
            .unwrap_or(&ev.func);
        let truncated = if detail.len() > 65 {
            format!("{}...", &detail[..62])
        } else {
            detail.to_string()
        };
        println!("{:<15.1} {}", ev.ts, truncated);
    }
    println!("\n{} budget violations.", violations.len());
    Ok(())
}

/// Parse HH:MM time string to milliseconds offset.
fn parse_hhmm_to_ms(time: &str) -> Result<f64> {
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 2 {
        return Err(eyre!("Invalid time format '{time}'. Use HH:MM."));
    }
    let hours: f64 = parts[0]
        .parse()
        .map_err(|_| eyre!("Invalid hours in '{time}'"))?;
    let mins: f64 = parts[1]
        .parse()
        .map_err(|_| eyre!("Invalid minutes in '{time}'"))?;
    Ok((hours * 3600.0 + mins * 60.0) * 1000.0)
}

/// Load events from the latest session file.
fn load_latest_session(log_dir: &Path) -> Result<Vec<TraceEvent>> {
    let sessions = store::list_sessions(log_dir)?;
    let path = sessions
        .first()
        .ok_or_else(|| eyre!("No session files found"))?;
    store::load_session(path)
}

/// Format bytes as a human-readable string.
fn format_bytes(bytes: i64) -> String {
    let abs = bytes.unsigned_abs();
    let sign = if bytes < 0 { "-" } else { "" };
    if abs >= 1_073_741_824 {
        format!("{sign}{:.1}GB", abs as f64 / 1_073_741_824.0)
    } else if abs >= 1_048_576 {
        format!("{sign}{:.1}MB", abs as f64 / 1_048_576.0)
    } else if abs >= 1024 {
        format!("{sign}{:.1}KB", abs as f64 / 1024.0)
    } else {
        format!("{sign}{abs}B")
    }
}
