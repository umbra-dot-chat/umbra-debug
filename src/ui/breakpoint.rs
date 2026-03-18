//! Conditional breakpoint system for pausing event ingestion.
//!
//! Breakpoints pause the event stream when a condition is met,
//! allowing step-by-step inspection. The breakpoint dialog is
//! toggled with `b`, and conditions are entered as text.
//!
//! Supported condition formats:
//! - `render>N`   — pause when render rate exceeds N/sec
//! - `heap>N`     — pause when heap exceeds N% (e.g. `heap>80`)
//! - `wasm>Nms`   — pause when a WASM call exceeds N milliseconds
//! - `error`      — pause on any error event
//! - `cat:level`  — pause on category:level match (e.g. `render:warn`)

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::app::{App, TraceEvent};

/// A conditional breakpoint that can pause event ingestion.
#[derive(Debug, Clone)]
pub struct Breakpoint {
    pub condition: BreakpointCondition,
    pub enabled: bool,
}

/// The condition that triggers a breakpoint.
#[derive(Debug, Clone)]
pub enum BreakpointCondition {
    /// Pause when render rate exceeds N events/sec.
    RenderRate(f64),
    /// Pause when heap usage exceeds N percent.
    HeapPercent(f64),
    /// Pause when any WASM call duration exceeds N milliseconds.
    WasmDuration(f64),
    /// Pause on any error event.
    ErrorEvent,
    /// Pause when an event matches category:level.
    CategoryLevel(String, String),
}

impl Breakpoint {
    /// Check whether this breakpoint should trigger on the given event.
    pub fn should_trigger(&self, event: &TraceEvent, app: &App) -> Option<String> {
        if !self.enabled {
            return None;
        }

        match &self.condition {
            BreakpointCondition::RenderRate(threshold) => {
                // Count render events in the last 10s
                if let Some(latest_ts) = app.events.last().map(|e| e.ts) {
                    let cutoff = latest_ts - 10_000.0;
                    let render_count = app
                        .events
                        .iter()
                        .rev()
                        .take_while(|e| e.ts > cutoff)
                        .filter(|e| e.cat == "render")
                        .count();
                    let rate = render_count as f64 / 10.0;
                    if rate > *threshold {
                        return Some(format!("Render rate {rate:.1}/s > {threshold}/s"));
                    }
                }
                None
            }
            BreakpointCondition::HeapPercent(threshold) => {
                // Use device_memory from first connected client as baseline
                let device_mem_bytes = app
                    .clients
                    .values()
                    .next()
                    .map(|c| (c.device_memory * 1_073_741_824.0) as i64)
                    .unwrap_or(4_294_967_296); // default 4GB

                if device_mem_bytes > 0 && event.mem_after > 0 {
                    let pct = (event.mem_after as f64 / device_mem_bytes as f64) * 100.0;
                    if pct > *threshold {
                        return Some(format!("Heap {pct:.1}% > {threshold}%"));
                    }
                }
                None
            }
            BreakpointCondition::WasmDuration(threshold_ms) => {
                if event.cat == "wasm" && event.dur_ms > *threshold_ms {
                    return Some(format!(
                        "WASM {} took {:.1}ms > {threshold_ms}ms",
                        event.func, event.dur_ms
                    ));
                }
                None
            }
            BreakpointCondition::ErrorEvent => {
                if event.err.is_some() || event.cat == "err" {
                    let msg = event
                        .err
                        .as_deref()
                        .unwrap_or(&event.func);
                    return Some(format!("Error: {}", &msg[..msg.len().min(60)]));
                }
                None
            }
            BreakpointCondition::CategoryLevel(cat, level) => {
                if event.cat == *cat {
                    if let Some(ref meta) = event.meta {
                        if meta.level == *level {
                            return Some(format!("Match {cat}:{level} on {}", event.func));
                        }
                    }
                }
                None
            }
        }
    }
}

/// Parse a breakpoint condition from user input text.
///
/// Supported formats:
/// - `render>50`   -> RenderRate(50.0)
/// - `heap>80`     -> HeapPercent(80.0)
/// - `wasm>100ms`  -> WasmDuration(100.0)
/// - `wasm>100`    -> WasmDuration(100.0)
/// - `error`       -> ErrorEvent
/// - `render:warn` -> CategoryLevel("render", "warn")
pub fn parse_condition(input: &str) -> Option<Breakpoint> {
    let input = input.trim();

    if input.eq_ignore_ascii_case("error") {
        return Some(Breakpoint {
            condition: BreakpointCondition::ErrorEvent,
            enabled: true,
        });
    }

    // category:level pattern (e.g., "render:warn")
    if let Some((cat, level)) = input.split_once(':') {
        let cat = cat.trim().to_string();
        let level = level.trim().to_string();
        if !cat.is_empty() && !level.is_empty() {
            return Some(Breakpoint {
                condition: BreakpointCondition::CategoryLevel(cat, level),
                enabled: true,
            });
        }
    }

    // threshold patterns: render>N, heap>N, wasm>Nms
    if let Some((prefix, value_str)) = input.split_once('>') {
        let prefix = prefix.trim().to_lowercase();
        let value_str = value_str.trim().trim_end_matches("ms");
        if let Ok(value) = value_str.parse::<f64>() {
            let condition = match prefix.as_str() {
                "render" => BreakpointCondition::RenderRate(value),
                "heap" => BreakpointCondition::HeapPercent(value),
                "wasm" => BreakpointCondition::WasmDuration(value),
                _ => return None,
            };
            return Some(Breakpoint {
                condition,
                enabled: true,
            });
        }
    }

    None
}

/// Render the breakpoint input dialog overlay.
pub fn render_breakpoint_dialog(frame: &mut Frame, input: &str, area: Rect) {
    // Position dialog in the center of the content area
    let dialog_width = 50u16.min(area.width.saturating_sub(4));
    let dialog_height = 5u16;
    let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
    let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

    // Clear background
    let bg = Paragraph::new("").style(Style::default().bg(Color::Black));
    frame.render_widget(bg, dialog_area);

    let lines = vec![
        Line::from(Span::styled(
            " Add Breakpoint ",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(Span::styled(
            " render>N  heap>N  wasm>Nms  error  cat:level ",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" > ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("{input}_"),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    let dialog = Paragraph::new(lines).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(dialog, dialog_area);
}

/// Render the pause indicator in the status bar area.
pub fn render_pause_indicator(frame: &mut Frame, reason: &str, area: Rect) {
    let text = format!(" PAUSED: {reason} [Space=resume, n=step, c=continue] ");
    let indicator = Paragraph::new(text).style(
        Style::default()
            .fg(Color::White)
            .bg(Color::Red)
            .bold(),
    );
    frame.render_widget(indicator, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_condition() {
        let bp = parse_condition("error").unwrap();
        assert!(matches!(bp.condition, BreakpointCondition::ErrorEvent));
        assert!(bp.enabled);
    }

    #[test]
    fn parse_render_rate() {
        let bp = parse_condition("render>50").unwrap();
        match bp.condition {
            BreakpointCondition::RenderRate(v) => assert!((v - 50.0).abs() < f64::EPSILON),
            _ => panic!("Expected RenderRate"),
        }
    }

    #[test]
    fn parse_heap_percent() {
        let bp = parse_condition("heap>80").unwrap();
        match bp.condition {
            BreakpointCondition::HeapPercent(v) => assert!((v - 80.0).abs() < f64::EPSILON),
            _ => panic!("Expected HeapPercent"),
        }
    }

    #[test]
    fn parse_wasm_duration_with_ms_suffix() {
        let bp = parse_condition("wasm>100ms").unwrap();
        match bp.condition {
            BreakpointCondition::WasmDuration(v) => assert!((v - 100.0).abs() < f64::EPSILON),
            _ => panic!("Expected WasmDuration"),
        }
    }

    #[test]
    fn parse_wasm_duration_without_suffix() {
        let bp = parse_condition("wasm>200").unwrap();
        match bp.condition {
            BreakpointCondition::WasmDuration(v) => assert!((v - 200.0).abs() < f64::EPSILON),
            _ => panic!("Expected WasmDuration"),
        }
    }

    #[test]
    fn parse_category_level() {
        let bp = parse_condition("render:warn").unwrap();
        match bp.condition {
            BreakpointCondition::CategoryLevel(cat, level) => {
                assert_eq!(cat, "render");
                assert_eq!(level, "warn");
            }
            _ => panic!("Expected CategoryLevel"),
        }
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(parse_condition("").is_none());
        assert!(parse_condition("unknown>50").is_none());
    }
}
