//! All Events tab — scrollable list of all trace events with color coding.
//!
//! Colors by category: wasm=cyan, sql=yellow, net=green, mem=magenta, err=red.
//! Memory growth is colored by severity: green(0), yellow(>64KB), red(>1MB).

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::app::App;
use super::format_bytes;

/// Render the All Events tab.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" All Events ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let events = app.filtered_events();

    if events.is_empty() {
        let empty = ratatui::widgets::Paragraph::new(
            "No events yet. Connect a browser to start receiving traces.",
        )
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let visible_height = inner.height as usize;

    // Calculate scroll position
    let total = events.len();
    let offset = if app.auto_scroll {
        total.saturating_sub(visible_height)
    } else {
        app.scroll_offset.min(total.saturating_sub(visible_height))
    };

    let visible_events = &events[offset..total.min(offset + visible_height)];

    let items: Vec<ListItem> = visible_events
        .iter()
        .map(|ev| {
            let cat_color = cat_color(&ev.cat);
            let growth_color = mem_growth_color(ev.mem_growth);

            let growth_str = if ev.mem_growth != 0 {
                format!(" {}", format_bytes(ev.mem_growth))
            } else {
                String::new()
            };

            let err_str = if let Some(ref err) = ev.err {
                format!(" ERR: {err}")
            } else {
                String::new()
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("[{:>10.1}] ", ev.ts),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("[{:<4}] ", ev.cat),
                    Style::default().fg(cat_color),
                ),
                Span::styled(
                    ev.func.clone(),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!(" {:.1}ms", ev.dur_ms),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(growth_str, Style::default().fg(growth_color)),
                Span::styled(err_str, Style::default().fg(Color::Red)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Get the display color for an event category.
fn cat_color(cat: &str) -> Color {
    match cat {
        "wasm" => Color::Cyan,
        "sql" => Color::Yellow,
        "net" => Color::Green,
        "mem" => Color::Magenta,
        "err" => Color::Red,
        _ => Color::White,
    }
}

/// Get color based on memory growth severity.
fn mem_growth_color(growth: i64) -> Color {
    let abs = growth.unsigned_abs();
    if abs == 0 {
        Color::Green
    } else if abs > 10_485_760 {
        // >10MB: bold red
        Color::LightRed
    } else if abs > 1_048_576 {
        // >1MB: red
        Color::Red
    } else if abs > 65_536 {
        // >64KB: yellow
        Color::Yellow
    } else {
        Color::Green
    }
}
