//! Memory tab — sparkline graphs of memory usage over time.
//!
//! Displays WASM memory, growth rate, and current values.
//! Window size is adjustable with +/- keys (10s-300s).

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Sparkline};

use crate::app::App;
use super::format_bytes;

/// Render the Memory tab with sparkline graphs.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(format!(
            " Memory (window: {}s, +/- to adjust) ",
            app.mem_graph_window_secs
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.events.is_empty() {
        let empty = Paragraph::new("No memory data yet.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Split into sections
    let sections = Layout::vertical([
        Constraint::Length(1),  // WASM Memory label
        Constraint::Length(4),  // WASM Memory sparkline
        Constraint::Length(1),  // Growth rate label
        Constraint::Length(4),  // Growth rate sparkline
        Constraint::Length(1),  // Spacer
        Constraint::Min(2),    // Current values
    ])
    .split(inner);

    let window_ms = app.mem_graph_window_secs as f64 * 1000.0;
    let latest_ts = app.events.last().map(|e| e.ts).unwrap_or(0.0);
    let cutoff = latest_ts - window_ms;

    // Collect only memory snapshot events (cat: "mem") within the window
    let mem_events: Vec<&crate::app::TraceEvent> = app
        .events
        .iter()
        .filter(|e| e.ts > cutoff && e.cat == "mem" && e.mem_after > 0)
        .collect();

    // WASM Memory sparkline
    let mem_label = Paragraph::new(Line::from(vec![
        Span::styled("WASM Memory  ", Style::default().fg(Color::Magenta).bold()),
        Span::styled(
            format!("Current: {}", format_bytes(app.latest_mem())),
            Style::default().fg(Color::White),
        ),
    ]));
    frame.render_widget(mem_label, sections[0]);

    let width = sections[1].width as usize;
    let mem_data = bucket_data(&mem_events, cutoff, latest_ts, width, |e| {
        e.mem_after as u64
    });

    let sparkline = Sparkline::default()
        .data(&mem_data)
        .style(Style::default().fg(Color::Magenta));
    frame.render_widget(sparkline, sections[1]);

    // Growth rate sparkline
    let total_growth = app.total_mem_growth();
    let growth_label = Paragraph::new(Line::from(vec![
        Span::styled(
            "Growth Rate  ",
            Style::default().fg(Color::Yellow).bold(),
        ),
        Span::styled(
            format!("Total: {}", format_bytes(total_growth)),
            Style::default().fg(Color::White),
        ),
    ]));
    frame.render_widget(growth_label, sections[2]);

    let growth_data = bucket_data(&mem_events, cutoff, latest_ts, width, |e| {
        e.mem_growth.unsigned_abs()
    });

    let growth_sparkline = Sparkline::default()
        .data(&growth_data)
        .style(Style::default().fg(Color::Yellow));
    frame.render_widget(growth_sparkline, sections[3]);

    // Current values summary
    let summary_lines = vec![
        Line::from(vec![
            Span::styled("Events: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", app.events.len()),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("  |  Rate: {:.0} ev/s", app.events_per_sec),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("  |  Window: {} data points", mem_events.len()),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];
    let summary = Paragraph::new(summary_lines);
    frame.render_widget(summary, sections[5]);
}

/// Bucket events into `width` bins and apply a value extractor.
fn bucket_data(
    events: &[&crate::app::TraceEvent],
    start_ts: f64,
    end_ts: f64,
    width: usize,
    value_fn: fn(&crate::app::TraceEvent) -> u64,
) -> Vec<u64> {
    if events.is_empty() || width == 0 {
        return vec![0; width];
    }

    let range = (end_ts - start_ts).max(1.0);
    let mut buckets = vec![0u64; width];
    let mut counts = vec![0u64; width];

    for ev in events {
        let normalized = ((ev.ts - start_ts) / range * width as f64) as usize;
        let idx = normalized.min(width.saturating_sub(1));
        buckets[idx] += value_fn(ev);
        counts[idx] += 1;
    }

    // Average values per bucket
    for i in 0..width {
        if counts[i] > 0 {
            buckets[i] /= counts[i];
        }
    }

    buckets
}
