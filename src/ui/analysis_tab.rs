//! Analysis tab — automated insights and anomaly detection.
//!
//! Sections: hot functions, memory suspects, repeat detection,
//! growth rate, and crash timeline.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::app::App;
use super::format_bytes;

/// Render the Analysis tab.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Analysis ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightBlue));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.events.is_empty() {
        let empty = Paragraph::new("No data to analyze yet.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Split into sections
    let sections = Layout::vertical([
        Constraint::Length(1),   // Hot functions header
        Constraint::Length(8),   // Hot functions table
        Constraint::Length(1),   // Memory suspects header
        Constraint::Length(8),   // Memory suspects table
        Constraint::Length(1),   // Anomalies header
        Constraint::Min(3),     // Anomalies list
    ])
    .split(inner);

    // 1. Hot functions (top 10 by calls/sec)
    render_hot_functions(frame, app, sections[0], sections[1]);

    // 2. Memory suspects (top 10 by total growth)
    render_memory_suspects(frame, app, sections[2], sections[3]);

    // 3. Anomalies: repeat detection + growth rate
    render_anomalies(frame, app, sections[4], sections[5]);
}

/// Top 10 functions by calls/sec.
fn render_hot_functions(
    frame: &mut Frame,
    app: &App,
    header_area: Rect,
    table_area: Rect,
) {
    let header = Paragraph::new(Span::styled(
        " Hot Functions (by calls/sec in last 10s) ",
        Style::default().fg(Color::LightRed).bold(),
    ));
    frame.render_widget(header, header_area);

    let mut funcs: Vec<(&String, f64)> = app
        .func_stats
        .iter()
        .map(|(name, stats)| (name, stats.recent_calls.len() as f64 / 10.0))
        .collect();
    funcs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let rows: Vec<Row> = funcs
        .iter()
        .take(10)
        .map(|(name, rate)| {
            let color = if *rate > 100.0 {
                Color::Red
            } else if *rate > 10.0 {
                Color::Yellow
            } else {
                Color::White
            };

            Row::new(vec![
                Cell::from(name.to_string()).style(Style::default().fg(color)),
                Cell::from(format!("{rate:.1}/s")).style(Style::default().fg(color)),
            ])
        })
        .collect();

    let widths = [Constraint::Min(40), Constraint::Length(12)];
    let table = Table::new(rows, widths);
    frame.render_widget(table, table_area);
}

/// Top 10 functions by total memory growth.
fn render_memory_suspects(
    frame: &mut Frame,
    app: &App,
    header_area: Rect,
    table_area: Rect,
) {
    let header = Paragraph::new(Span::styled(
        " Memory Suspects (by total growth) ",
        Style::default().fg(Color::Magenta).bold(),
    ));
    frame.render_widget(header, header_area);

    let mut suspects: Vec<(&String, i64)> = app
        .func_stats
        .iter()
        .filter(|(_, stats)| stats.total_mem_growth > 0)
        .map(|(name, stats)| (name, stats.total_mem_growth))
        .collect();
    suspects.sort_by(|a, b| b.1.cmp(&a.1));

    let rows: Vec<Row> = suspects
        .iter()
        .take(10)
        .map(|(name, growth)| {
            let color = if *growth > 10_485_760 {
                Color::Red
            } else if *growth > 1_048_576 {
                Color::Yellow
            } else {
                Color::Green
            };

            Row::new(vec![
                Cell::from(name.to_string()).style(Style::default().fg(color)),
                Cell::from(format_bytes(*growth)).style(Style::default().fg(color)),
            ])
        })
        .collect();

    let widths = [Constraint::Min(40), Constraint::Length(12)];
    let table = Table::new(rows, widths);
    frame.render_widget(table, table_area);
}

/// Anomaly detection: repeat functions (>100/s) and growth rate.
fn render_anomalies(
    frame: &mut Frame,
    app: &App,
    header_area: Rect,
    content_area: Rect,
) {
    let header = Paragraph::new(Span::styled(
        " Anomalies ",
        Style::default().fg(Color::LightRed).bold(),
    ));
    frame.render_widget(header, header_area);

    let mut lines: Vec<Line> = Vec::new();

    // Repeat detection: functions called >100 times/sec
    let repeaters: Vec<(&String, f64)> = app
        .func_stats
        .iter()
        .map(|(name, stats)| (name, stats.recent_calls.len() as f64 / 10.0))
        .filter(|(_, rate)| *rate > 100.0)
        .collect();

    if !repeaters.is_empty() {
        lines.push(Line::from(Span::styled(
            "  REPEAT WARNING: Functions called >100/s:",
            Style::default().fg(Color::Red),
        )));
        for (name, rate) in &repeaters {
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    format!("{name}: {rate:.0}/s"),
                    Style::default().fg(Color::Red),
                ),
            ]));
        }
    }

    // Growth rate
    let growth_rate = calculate_growth_rate(app);
    let growth_color = if growth_rate > 1_048_576.0 {
        Color::Red
    } else if growth_rate > 65_536.0 {
        Color::Yellow
    } else {
        Color::Green
    };

    lines.push(Line::from(vec![
        Span::styled("  Growth rate: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}/s", format_bytes(growth_rate as i64)),
            Style::default().fg(growth_color),
        ),
    ]));

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No anomalies detected.",
            Style::default().fg(Color::Green),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, content_area);
}

/// Calculate current memory growth rate in bytes/sec.
fn calculate_growth_rate(app: &App) -> f64 {
    let latest_ts = match app.events.last() {
        Some(ev) => ev.ts,
        None => return 0.0,
    };

    let window = 10_000.0; // 10 seconds
    let cutoff = latest_ts - window;

    let growth: i64 = app
        .events
        .iter()
        .filter(|e| e.ts > cutoff)
        .map(|e| e.mem_growth)
        .sum();

    growth as f64 / (window / 1000.0)
}
