//! Log tab — scrollable, filterable list of application log entries.
//!
//! Displays log entries from the browser debug bridge with color-coded
//! levels: trace=DarkGray, debug=Cyan, info=Green, warn=Yellow,
//! error=Red, fatal=Red+Bold.
//!
//! Keybindings: c=cycle category, l=cycle level, s=source filter,
//! j/k=scroll, G=bottom, g=top, x=clear.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::App;

/// Render the Log tab.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Log ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.log_entries.is_empty() {
        let help = vec![
            Line::from(""),
            Line::from(Span::styled(
                "No log entries received yet.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Keybindings:",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  c = cycle category filter    l = cycle level filter",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "  s = set/clear source filter  x = clear all entries",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "  j/k = scroll up/down   G = bottom   g = top",
                Style::default().fg(Color::Yellow),
            )),
        ];
        let empty = Paragraph::new(help).alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Reserve 1 line at the bottom for the log status bar
    if inner.height < 2 {
        return;
    }
    let content_area = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let status_area = Rect {
        y: inner.y + content_area.height,
        height: 1,
        ..inner
    };

    let entries = app.filtered_log_entries();

    if entries.is_empty() {
        let empty = Paragraph::new("No entries match current filters.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, content_area);
        render_log_status_bar(frame, app, 0, 0, status_area);
        return;
    }

    let visible_height = content_area.height as usize;
    let total = entries.len();

    // Calculate scroll position
    let offset = if app.log_auto_scroll {
        total.saturating_sub(visible_height)
    } else {
        app.log_scroll_offset.min(total.saturating_sub(visible_height))
    };

    let end = total.min(offset + visible_height);
    let visible_entries = &entries[offset..end];

    let items: Vec<ListItem> = visible_entries
        .iter()
        .map(|entry| {
            let level_style = level_style(&entry.level);

            // Format timestamp as HH:MM:SS.mmm from performance.now() ms
            let total_secs = (entry.timestamp / 1000.0) as u64;
            let hours = (total_secs / 3600) % 24;
            let mins = (total_secs / 60) % 60;
            let secs = total_secs % 60;
            let millis = (entry.timestamp % 1000.0) as u64;

            let data_suffix = if entry.data.is_empty() {
                String::new()
            } else {
                // Truncate data to keep lines readable
                let truncated = if entry.data.len() > 80 {
                    format!("{}...", &entry.data[..77])
                } else {
                    entry.data.clone()
                };
                format!(" | {truncated}")
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("{hours:02}:{mins:02}:{secs:02}.{millis:03} "),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("[{:<5}] ", entry.level.to_uppercase()),
                    level_style,
                ),
                Span::styled(
                    format!("[{:<13}] ", truncate_pad(&entry.category, 13)),
                    Style::default().fg(Color::Blue),
                ),
                Span::styled(
                    format!("[{:<14}] ", truncate_pad(&entry.source, 14)),
                    Style::default().fg(Color::Magenta),
                ),
                Span::styled(
                    entry.message.clone(),
                    Style::default().fg(Color::White),
                ),
                Span::styled(data_suffix, Style::default().fg(Color::DarkGray)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, content_area);

    render_log_status_bar(frame, app, total, offset, status_area);
}

/// Render the log-specific status bar at the bottom of the Log tab.
fn render_log_status_bar(
    frame: &mut Frame,
    app: &App,
    filtered_count: usize,
    scroll_offset: usize,
    area: Rect,
) {
    let total_count = app.log_entries.len();

    let mut parts = vec![format!("{filtered_count}/{total_count} entries")];

    // Show active filters
    if let Some(ref level) = app.log_level_filter {
        parts.push(format!("level>={level}"));
    }
    if let Some(ref cat) = app.log_category_filter {
        parts.push(format!("cat={cat}"));
    }
    if let Some(ref src) = app.log_source_filter {
        parts.push(format!("src={src}"));
    }

    // Scroll position
    if filtered_count > 0 {
        if app.log_auto_scroll {
            parts.push("TAIL".to_string());
        } else {
            parts.push(format!("@{scroll_offset}"));
        }
    }

    // Source input mode indicator
    if app.log_source_input_mode {
        parts.push(format!("src>{}|", app.log_source_input));
    }

    let status_text = parts.join(" | ");
    let bar = Paragraph::new(status_text).style(
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray),
    );
    frame.render_widget(bar, area);
}

/// Get the display style for a log level.
fn level_style(level: &str) -> Style {
    match level {
        "trace" => Style::default().fg(Color::DarkGray),
        "debug" => Style::default().fg(Color::Cyan),
        "info" => Style::default().fg(Color::Green),
        "warn" => Style::default().fg(Color::Yellow),
        "error" => Style::default().fg(Color::Red),
        "fatal" => Style::default().fg(Color::Red).bold(),
        _ => Style::default().fg(Color::White),
    }
}

/// Truncate a string to max_len, padding with spaces if shorter.
fn truncate_pad(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}~", &s[..max_len - 1])
    } else {
        format!("{s:<width$}", width = max_len)
    }
}
