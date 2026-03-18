//! WASM tab — per-function statistics table with sparklines.
//!
//! Shows call count, calls/sec, total duration, avg duration,
//! and total memory growth for each traced WASM function.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};

use crate::app::App;
use super::format_bytes;

/// Render the WASM function stats table.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" WASM Functions ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.func_stats.is_empty() {
        let empty = ratatui::widgets::Paragraph::new("No WASM function data yet.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Collect and sort by call count descending
    let mut rows: Vec<(&String, &crate::app::FuncStats)> =
        app.func_stats.iter().collect();
    rows.sort_by(|a, b| b.1.call_count.cmp(&a.1.call_count));

    // Header
    let header = Row::new(vec![
        Cell::from("Function").style(Style::default().fg(Color::Cyan).bold()),
        Cell::from("Calls").style(Style::default().fg(Color::Cyan).bold()),
        Cell::from("Calls/s").style(Style::default().fg(Color::Cyan).bold()),
        Cell::from("Total ms").style(Style::default().fg(Color::Cyan).bold()),
        Cell::from("Avg ms").style(Style::default().fg(Color::Cyan).bold()),
        Cell::from("Mem Growth").style(Style::default().fg(Color::Cyan).bold()),
    ])
    .height(1);

    let visible_height = inner.height.saturating_sub(2) as usize;
    let offset = app.scroll_offset.min(rows.len().saturating_sub(visible_height));
    let visible_rows = &rows[offset..rows.len().min(offset + visible_height)];

    let data_rows: Vec<Row> = visible_rows
        .iter()
        .map(|(func_name, stats)| {
            let calls_per_sec = stats.recent_calls.len() as f64 / 10.0;
            let avg_dur = if stats.call_count > 0 {
                stats.total_dur_ms / stats.call_count as f64
            } else {
                0.0
            };

            let growth_color = mem_growth_row_color(stats.total_mem_growth);

            Row::new(vec![
                Cell::from(truncate(func_name, 40))
                    .style(Style::default().fg(Color::White)),
                Cell::from(format!("{}", stats.call_count))
                    .style(Style::default().fg(Color::White)),
                Cell::from(format!("{:.1}", calls_per_sec))
                    .style(Style::default().fg(Color::White)),
                Cell::from(format!("{:.1}", stats.total_dur_ms))
                    .style(Style::default().fg(Color::White)),
                Cell::from(format!("{:.2}", avg_dur))
                    .style(Style::default().fg(Color::White)),
                Cell::from(format_bytes(stats.total_mem_growth))
                    .style(Style::default().fg(growth_color)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(30),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(12),
    ];

    let table = Table::new(data_rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(Color::DarkGray));

    frame.render_widget(table, inner);
}

/// Color for row based on total memory growth.
fn mem_growth_row_color(growth: i64) -> Color {
    let abs = growth.unsigned_abs();
    if abs > 10_485_760 {
        Color::Red
    } else if abs > 1_048_576 {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// Truncate a string to max_len, appending "..." if needed.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}
