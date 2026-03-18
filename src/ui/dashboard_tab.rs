//! Dashboard tab — overview of key metrics at a glance.
//!
//! Three rows:
//! - Top: sparklines for Heap Memory, Render Rate, Events/sec
//! - Middle: Hot Functions, Memory Suspects, Active Errors
//! - Bottom: Budget Status, Web Vitals, Session Info

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Sparkline, Table};

use crate::app::App;
use super::format_bytes;

/// Render the Dashboard tab.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Dashboard ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.events.is_empty() {
        let empty = Paragraph::new("Waiting for trace events...")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Three-row layout: sparklines | tables | status
    let rows = Layout::vertical([
        Constraint::Length(6),  // Sparklines row
        Constraint::Min(8),    // Tables row
        Constraint::Length(4), // Status row
    ])
    .split(inner);

    render_sparklines_row(frame, app, rows[0]);
    render_tables_row(frame, app, rows[1]);
    render_status_row(frame, app, rows[2]);
}

// ---------------------------------------------------------------------------
// Top row: three sparkline columns
// ---------------------------------------------------------------------------

fn render_sparklines_row(frame: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(area);

    render_heap_sparkline(frame, app, cols[0]);
    render_render_rate_sparkline(frame, app, cols[1]);
    render_events_sparkline(frame, app, cols[2]);
}

fn render_heap_sparkline(frame: &mut Frame, app: &App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(1), // label
        Constraint::Min(1),   // sparkline
    ])
    .split(area);

    let current_heap = format_bytes(app.latest_mem());
    let label = Paragraph::new(Line::from(vec![
        Span::styled("Heap ", Style::default().fg(Color::Magenta).bold()),
        Span::styled(current_heap, Style::default().fg(Color::White)),
    ]));
    frame.render_widget(label, sections[0]);

    let width = sections[1].width as usize;
    let data = build_sparkline_data(app, width, |e| {
        if e.mem_after > 0 { e.mem_after as u64 } else { 0 }
    });

    let sparkline = Sparkline::default()
        .data(&data)
        .style(Style::default().fg(Color::Magenta));
    frame.render_widget(sparkline, sections[1]);
}

fn render_render_rate_sparkline(frame: &mut Frame, app: &App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    // Count render events in the last 60s window, bucketed
    let render_count: usize = app
        .events
        .iter()
        .rev()
        .take_while(|e| {
            app.events
                .last()
                .map(|last| last.ts - e.ts < 60_000.0)
                .unwrap_or(false)
        })
        .filter(|e| e.cat == "render")
        .count();

    let label = Paragraph::new(Line::from(vec![
        Span::styled("Renders ", Style::default().fg(Color::Green).bold()),
        Span::styled(
            format!("{render_count}/60s"),
            Style::default().fg(Color::White),
        ),
    ]));
    frame.render_widget(label, sections[0]);

    let width = sections[1].width as usize;
    let data = build_sparkline_data(app, width, |e| {
        if e.cat == "render" { 1 } else { 0 }
    });

    let sparkline = Sparkline::default()
        .data(&data)
        .style(Style::default().fg(Color::Green));
    frame.render_widget(sparkline, sections[1]);
}

fn render_events_sparkline(frame: &mut Frame, app: &App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    let label = Paragraph::new(Line::from(vec![
        Span::styled("Events ", Style::default().fg(Color::Yellow).bold()),
        Span::styled(
            format!("{:.0}/s", app.events_per_sec),
            Style::default().fg(Color::White),
        ),
    ]));
    frame.render_widget(label, sections[0]);

    let width = sections[1].width as usize;
    // Bucket all events (count per bucket)
    let data = build_sparkline_data(app, width, |_| 1);

    let sparkline = Sparkline::default()
        .data(&data)
        .style(Style::default().fg(Color::Yellow));
    frame.render_widget(sparkline, sections[1]);
}

/// Build sparkline data by bucketing recent events (last 60s) into `width` bins.
fn build_sparkline_data(
    app: &App,
    width: usize,
    value_fn: fn(&crate::app::TraceEvent) -> u64,
) -> Vec<u64> {
    if app.events.is_empty() || width == 0 {
        return vec![0; width.max(1)];
    }

    let latest_ts = app.events.last().unwrap().ts;
    let window_ms = 60_000.0_f64;
    let start_ts = latest_ts - window_ms;
    let range = window_ms;

    let mut buckets = vec![0u64; width];

    for ev in app.events.iter().rev() {
        if ev.ts < start_ts {
            break;
        }
        let normalized = ((ev.ts - start_ts) / range * width as f64) as usize;
        let idx = normalized.min(width.saturating_sub(1));
        buckets[idx] += value_fn(ev);
    }

    buckets
}

// ---------------------------------------------------------------------------
// Middle row: Hot Functions | Memory Suspects | Active Errors
// ---------------------------------------------------------------------------

fn render_tables_row(frame: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(area);

    render_hot_functions(frame, app, cols[0]);
    render_memory_suspects(frame, app, cols[1]);
    render_active_errors(frame, app, cols[2]);
}

fn render_hot_functions(frame: &mut Frame, app: &App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    let header = Paragraph::new(Span::styled(
        " Hot Functions ",
        Style::default().fg(Color::LightRed).bold(),
    ));
    frame.render_widget(header, sections[0]);

    let mut funcs: Vec<(&String, f64)> = app
        .func_stats
        .iter()
        .map(|(name, stats)| (name, stats.recent_calls.len() as f64 / 10.0))
        .collect();
    funcs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let rows: Vec<Row> = funcs
        .iter()
        .take(5)
        .map(|(name, rate)| {
            let color = if *rate > 100.0 {
                Color::Red
            } else if *rate > 10.0 {
                Color::Yellow
            } else {
                Color::White
            };
            // Truncate long function names to fit
            let display_name = truncate_str(name, 20);
            Row::new(vec![
                Cell::from(display_name).style(Style::default().fg(color)),
                Cell::from(format!("{rate:.1}/s")).style(Style::default().fg(color)),
            ])
        })
        .collect();

    let widths = [Constraint::Min(10), Constraint::Length(8)];
    let table = Table::new(rows, widths);
    frame.render_widget(table, sections[1]);
}

fn render_memory_suspects(frame: &mut Frame, app: &App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    let header = Paragraph::new(Span::styled(
        " Mem Suspects ",
        Style::default().fg(Color::Magenta).bold(),
    ));
    frame.render_widget(header, sections[0]);

    let mut suspects: Vec<(&String, i64)> = app
        .func_stats
        .iter()
        .filter(|(_, stats)| stats.total_mem_growth > 0)
        .map(|(name, stats)| (name, stats.total_mem_growth))
        .collect();
    suspects.sort_by(|a, b| b.1.cmp(&a.1));

    let rows: Vec<Row> = suspects
        .iter()
        .take(5)
        .map(|(name, growth)| {
            let color = if *growth > 10_485_760 {
                Color::Red
            } else if *growth > 1_048_576 {
                Color::Yellow
            } else {
                Color::Green
            };
            let display_name = truncate_str(name, 20);
            Row::new(vec![
                Cell::from(display_name).style(Style::default().fg(color)),
                Cell::from(format_bytes(*growth)).style(Style::default().fg(color)),
            ])
        })
        .collect();

    let widths = [Constraint::Min(10), Constraint::Length(8)];
    let table = Table::new(rows, widths);
    frame.render_widget(table, sections[1]);
}

fn render_active_errors(frame: &mut Frame, app: &App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    let header = Paragraph::new(Span::styled(
        " Active Errors ",
        Style::default().fg(Color::Red).bold(),
    ));
    frame.render_widget(header, sections[0]);

    let error_events: Vec<&crate::app::TraceEvent> = app
        .events
        .iter()
        .rev()
        .filter(|e| e.err.is_some() || e.cat == "err")
        .take(3)
        .collect();

    if error_events.is_empty() {
        let ok = Paragraph::new(Span::styled(
            "  No errors",
            Style::default().fg(Color::Green),
        ));
        frame.render_widget(ok, sections[1]);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for ev in &error_events {
        let err_msg = ev
            .err
            .as_deref()
            .unwrap_or(&ev.func);
        let display = truncate_str(err_msg, 35);
        lines.push(Line::from(Span::styled(
            format!("  {display}"),
            Style::default().fg(Color::Red),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, sections[1]);
}

// ---------------------------------------------------------------------------
// Bottom row: Budget Status | Web Vitals | Session Info
// ---------------------------------------------------------------------------

fn render_status_row(frame: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(area);

    render_budget_status(frame, app, cols[0]);
    render_web_vitals(frame, app, cols[1]);
    render_session_info(frame, app, cols[2]);
}

fn render_budget_status(frame: &mut Frame, app: &App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    let header = Paragraph::new(Span::styled(
        " Budgets ",
        Style::default().fg(Color::Cyan).bold(),
    ));
    frame.render_widget(header, sections[0]);

    // Check for budget violations in event stream
    let has_render_violation = app.events.iter().rev().take(500).any(|e| {
        e.func.contains("BUDGET EXCEEDED") && e.cat == "render"
    });
    let has_mem_violation = app.events.iter().rev().take(500).any(|e| {
        e.func.contains("BUDGET EXCEEDED") && e.cat == "mem"
    });
    let has_net_violation = app.events.iter().rev().take(500).any(|e| {
        e.func.contains("BUDGET EXCEEDED") && e.cat == "net"
    });

    let check = |ok: bool| if ok { ("OK", Color::Green) } else { ("!!", Color::Red) };
    let (r_sym, r_col) = check(!has_render_violation);
    let (m_sym, m_col) = check(!has_mem_violation);
    let (n_sym, n_col) = check(!has_net_violation);

    let line = Line::from(vec![
        Span::styled("  Render:", Style::default().fg(Color::DarkGray)),
        Span::styled(r_sym, Style::default().fg(r_col)),
        Span::styled(" Mem:", Style::default().fg(Color::DarkGray)),
        Span::styled(m_sym, Style::default().fg(m_col)),
        Span::styled(" Net:", Style::default().fg(Color::DarkGray)),
        Span::styled(n_sym, Style::default().fg(n_col)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, sections[1]);
}

fn render_web_vitals(frame: &mut Frame, app: &App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    let header = Paragraph::new(Span::styled(
        " Web Vitals ",
        Style::default().fg(Color::LightBlue).bold(),
    ));
    frame.render_widget(header, sections[0]);

    // Extract vitals from latest heartbeat meta fields
    // Heartbeats arrive as browser events with meta data containing vitals
    let latest_heartbeat = app.events.iter().rev().find(|e| {
        e.cat == "browser"
            && e.meta
                .as_ref()
                .is_some_and(|m| !m.data.is_empty())
    });

    let mut lines: Vec<Line> = Vec::new();

    if let Some(hb) = latest_heartbeat {
        if let Some(ref meta) = hb.meta {
            // Parse vitals from meta.data JSON if available
            let vitals = extract_vitals(&meta.data);
            lines.push(Line::from(vec![
                Span::styled("  INP:", Style::default().fg(Color::DarkGray)),
                vital_span(&vitals.inp, 200.0, 500.0),
                Span::styled(" CLS:", Style::default().fg(Color::DarkGray)),
                vital_span(&vitals.cls, 0.1, 0.25),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  LCP:", Style::default().fg(Color::DarkGray)),
                vital_span(&vitals.lcp, 2500.0, 4000.0),
                Span::styled(" FCP:", Style::default().fg(Color::DarkGray)),
                vital_span(&vitals.fcp, 1800.0, 3000.0),
            ]));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  No vitals data",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, sections[1]);
}

fn render_session_info(frame: &mut Frame, app: &App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    let header = Paragraph::new(Span::styled(
        " Session ",
        Style::default().fg(Color::White).bold(),
    ));
    frame.render_widget(header, sections[0]);

    let client_count = app.clients.len();
    let event_count = app.events.len();
    let duration = app.session_duration();

    let lines = vec![
        Line::from(vec![
            Span::styled("  Clients:", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{client_count}"),
                Style::default().fg(Color::White),
            ),
            Span::styled(" Events:", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{event_count}"),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Uptime:", Style::default().fg(Color::DarkGray)),
            Span::styled(duration, Style::default().fg(Color::White)),
        ]),
    ];

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, sections[1]);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Web Vitals values extracted from heartbeat meta data.
struct Vitals {
    inp: Option<f64>,
    cls: Option<f64>,
    lcp: Option<f64>,
    fcp: Option<f64>,
}

/// Try to extract web vitals from a JSON data string.
fn extract_vitals(data: &str) -> Vitals {
    let parsed: serde_json::Value =
        serde_json::from_str(data).unwrap_or(serde_json::Value::Null);

    Vitals {
        inp: parsed.get("inp").and_then(|v| v.as_f64()),
        cls: parsed.get("cls").and_then(|v| v.as_f64()),
        lcp: parsed.get("lcp").and_then(|v| v.as_f64()),
        fcp: parsed.get("fcp").and_then(|v| v.as_f64()),
    }
}

/// Color a vital value: green if good, yellow if needs improvement, red if poor.
fn vital_span(value: &Option<f64>, good_threshold: f64, poor_threshold: f64) -> Span<'static> {
    match value {
        Some(v) => {
            let color = if *v <= good_threshold {
                Color::Green
            } else if *v <= poor_threshold {
                Color::Yellow
            } else {
                Color::Red
            };
            Span::styled(format!("{v:.0}"), Style::default().fg(color))
        }
        None => Span::styled("--", Style::default().fg(Color::DarkGray)),
    }
}

/// Truncate a string to `max_len` characters, appending ".." if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}..", &s[..max_len.saturating_sub(2)])
    }
}
