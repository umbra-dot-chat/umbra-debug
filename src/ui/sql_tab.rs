//! SQL tab — query frequency, slow queries, and large exports.
//!
//! Three vertically-split sections showing top queries by frequency,
//! slowest queries, and largest data operations.

use std::collections::HashMap;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::app::{App, TraceEvent};

/// Render the SQL analysis tab.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" SQL Analysis ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sql_events = app.events_by_cat("sql");

    if sql_events.is_empty() {
        let empty = Paragraph::new("No SQL events yet.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Split into three vertical sections
    let sections = Layout::vertical([
        Constraint::Percentage(40), // Top queries
        Constraint::Percentage(30), // Slowest queries
        Constraint::Percentage(30), // Largest operations
    ])
    .split(inner);

    render_top_queries(frame, &sql_events, sections[0]);
    render_slow_queries(frame, &sql_events, sections[1]);
    render_large_ops(frame, &sql_events, sections[2]);
}

/// Top queries by frequency.
fn render_top_queries(frame: &mut Frame, events: &[&TraceEvent], area: Rect) {
    let title = Paragraph::new(Line::from(Span::styled(
        " Top Queries (by frequency) ",
        Style::default().fg(Color::Yellow).bold(),
    )));
    let title_area = Rect { height: 1, ..area };
    frame.render_widget(title, title_area);

    let table_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };

    // Aggregate by function name
    let mut freq_map: HashMap<&str, (usize, f64)> = HashMap::new();
    for ev in events {
        let entry = freq_map.entry(&ev.func).or_default();
        entry.0 += 1;
        entry.1 += ev.dur_ms;
    }

    let mut sorted: Vec<(&str, usize, f64)> = freq_map
        .into_iter()
        .map(|(f, (c, d))| (f, c, d))
        .collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let header = Row::new(vec![
        Cell::from("Statement").style(Style::default().fg(Color::Yellow)),
        Cell::from("Count").style(Style::default().fg(Color::Yellow)),
        Cell::from("Avg ms").style(Style::default().fg(Color::Yellow)),
        Cell::from("Caller").style(Style::default().fg(Color::Yellow)),
    ]);

    let rows: Vec<Row> = sorted
        .iter()
        .take(10)
        .map(|(func, count, total_dur)| {
            let avg = if *count > 0 {
                total_dur / *count as f64
            } else {
                0.0
            };
            // Find a caller context for this function
            let caller = events
                .iter()
                .find(|e| e.func == *func)
                .and_then(|e| e.sql_context.as_deref())
                .unwrap_or("-");

            Row::new(vec![
                Cell::from(func.to_string()),
                Cell::from(format!("{count}")),
                Cell::from(format!("{avg:.2}")),
                Cell::from(caller.to_string()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(25),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Min(20),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, table_area);
}

/// Slowest queries by max duration.
fn render_slow_queries(frame: &mut Frame, events: &[&TraceEvent], area: Rect) {
    let title = Paragraph::new(Line::from(Span::styled(
        " Slowest Queries ",
        Style::default().fg(Color::Red).bold(),
    )));
    let title_area = Rect { height: 1, ..area };
    frame.render_widget(title, title_area);

    let table_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };

    // Sort by duration descending
    let mut sorted: Vec<&&TraceEvent> = events.iter().collect();
    sorted.sort_by(|a, b| b.dur_ms.partial_cmp(&a.dur_ms).unwrap_or(std::cmp::Ordering::Equal));

    let header = Row::new(vec![
        Cell::from("Statement").style(Style::default().fg(Color::Red)),
        Cell::from("Duration ms").style(Style::default().fg(Color::Red)),
        Cell::from("Caller").style(Style::default().fg(Color::Red)),
    ]);

    let rows: Vec<Row> = sorted
        .iter()
        .take(8)
        .map(|ev| {
            let caller = ev.sql_context.as_deref().unwrap_or("-");
            Row::new(vec![
                Cell::from(ev.func.clone()),
                Cell::from(format!("{:.2}", ev.dur_ms)),
                Cell::from(caller.to_string()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(25),
        Constraint::Length(12),
        Constraint::Min(20),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, table_area);
}

/// Largest data operations by arg_bytes.
fn render_large_ops(frame: &mut Frame, events: &[&TraceEvent], area: Rect) {
    let title = Paragraph::new(Line::from(Span::styled(
        " Largest Operations ",
        Style::default().fg(Color::Magenta).bold(),
    )));
    let title_area = Rect { height: 1, ..area };
    frame.render_widget(title, title_area);

    let table_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };

    let mut sorted: Vec<&&TraceEvent> = events.iter().collect();
    sorted.sort_by(|a, b| b.arg_bytes.cmp(&a.arg_bytes));

    let header = Row::new(vec![
        Cell::from("Statement").style(Style::default().fg(Color::Magenta)),
        Cell::from("Size").style(Style::default().fg(Color::Magenta)),
        Cell::from("Duration ms").style(Style::default().fg(Color::Magenta)),
    ]);

    let rows: Vec<Row> = sorted
        .iter()
        .take(8)
        .map(|ev| {
            Row::new(vec![
                Cell::from(ev.func.clone()),
                Cell::from(super::format_bytes(ev.arg_bytes as i64)),
                Cell::from(format!("{:.2}", ev.dur_ms)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(30),
        Constraint::Length(12),
        Constraint::Length(12),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, table_area);
}
