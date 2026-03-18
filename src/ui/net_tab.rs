//! Network tab — timeline of network events.
//!
//! Shows WebSocket messages, connection state changes,
//! and offline batch markers with color coding by type.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::App;
use super::format_bytes;

/// Render the Network events tab.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Network Timeline ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let net_events = app.events_by_cat("net");

    if net_events.is_empty() {
        let empty = Paragraph::new("No network events yet.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let total = net_events.len();
    let offset = if app.auto_scroll {
        total.saturating_sub(visible_height)
    } else {
        app.scroll_offset.min(total.saturating_sub(visible_height))
    };

    let visible = &net_events[offset..total.min(offset + visible_height)];

    let items: Vec<ListItem> = visible
        .iter()
        .map(|ev| {
            let size_str = if ev.arg_bytes > 0 {
                format!(" {}", format_bytes(ev.arg_bytes as i64))
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
                    ev.func.clone(),
                    Style::default().fg(Color::Green),
                ),
                Span::styled(
                    format!(" {:.1}ms", ev.dur_ms),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(size_str, Style::default().fg(Color::White)),
                Span::styled(err_str, Style::default().fg(Color::Red)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}
