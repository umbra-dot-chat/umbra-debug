//! Error tab — errors and warnings with surrounding context.
//!
//! Shows each error event with +/- 5 surrounding events for
//! debugging context. Red for errors, yellow for warnings.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::App;

/// Render the Error tab.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Errors ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Find all error events
    let error_indices: Vec<usize> = app
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.err.is_some() || e.cat == "err")
        .map(|(i, _)| i)
        .collect();

    if error_indices.is_empty() {
        let empty = Paragraph::new("No errors detected.")
            .style(Style::default().fg(Color::Green))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Build display items: for each error, show it with +/-5 context
    let mut items: Vec<ListItem> = Vec::new();

    let visible_height = inner.height as usize;
    let total_errors = error_indices.len();

    // Show the most recent errors first (from the end)
    let start_err = if app.auto_scroll {
        total_errors.saturating_sub(3) // Show last ~3 errors with context
    } else {
        app.scroll_offset.min(total_errors.saturating_sub(1))
    };

    for &err_idx in error_indices.iter().skip(start_err) {
        if items.len() >= visible_height {
            break;
        }

        // Separator
        items.push(ListItem::new(Line::from(Span::styled(
            format!("--- Error #{} ---", err_idx),
            Style::default().fg(Color::Red).bold(),
        ))));

        // Context: 5 events before
        let context_start = err_idx.saturating_sub(5);
        let context_end = (err_idx + 6).min(app.events.len());

        for i in context_start..context_end {
            if items.len() >= visible_height {
                break;
            }

            let ev = &app.events[i];
            let is_error = i == err_idx;

            let style = if is_error {
                Style::default().fg(Color::Red).bold()
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let marker = if is_error { ">>" } else { "  " };
            let err_text = ev
                .err
                .as_deref()
                .map(|e| format!(" | {e}"))
                .unwrap_or_default();

            let line = Line::from(vec![
                Span::styled(marker, style),
                Span::styled(
                    format!(" [{:>10.1}] [{}] {} {:.1}ms{}",
                        ev.ts, ev.cat, ev.func, ev.dur_ms, err_text),
                    style,
                ),
            ]);

            items.push(ListItem::new(line));
        }

        // Blank line between error groups
        items.push(ListItem::new(Line::from("")));
    }

    let list = List::new(items);
    frame.render_widget(list, inner);
}
