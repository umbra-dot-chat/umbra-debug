//! Replay tab — time-travel debugging with cursor-based event navigation.
//!
//! Shows the event timeline with a movable cursor, reconstructed state
//! panel (render counts, memory, errors), bookmarks, and speed controls.
//!
//! Keybindings (when Replay tab is active):
//!   Left/Right = step backward/forward
//!   1/2/5/0    = set speed 1x/2x/5x/10x
//!   p          = toggle pause
//!   m          = toggle bookmark at cursor
//!   n/N        = next/prev bookmark
//!   /          = search within events (handled by app filter_mode)
//!   Enter      = expand/collapse event detail
//!   Home/End   = jump to start/end
//!   l          = load session file picker

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::App;
use crate::store::replay::ReplaySession;
use super::format_bytes;

/// Render the Replay tab.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Replay ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightYellow));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let session = match &app.replay_session {
        Some(s) => s,
        None => {
            render_no_session(frame, inner);
            return;
        }
    };

    if session.total_events() == 0 {
        let empty = Paragraph::new("Session file is empty.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Layout: events list (left 60%) | state panel (right 40%)
    // Bottom: status bar
    if inner.height < 3 {
        return;
    }

    let vert = Layout::vertical([
        Constraint::Min(5),    // Content
        Constraint::Length(1), // Status bar
    ])
    .split(inner);

    let content = vert[0];
    let status_area = vert[1];

    let horiz = Layout::horizontal([
        Constraint::Percentage(60), // Event list
        Constraint::Percentage(40), // State panel
    ])
    .split(content);

    render_event_list(frame, session, horiz[0]);
    render_state_panel(frame, session, horiz[1]);
    render_replay_status_bar(frame, session, status_area);
}

/// Render the "no session loaded" help screen.
fn render_no_session(frame: &mut Frame, area: Rect) {
    let help = vec![
        Line::from(""),
        Line::from(Span::styled(
            "No replay session loaded.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press 'l' to load a session file, or start with:",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  umbra-debug --replay <session.jsonl>",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Keybindings:",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Left/Right = step    1/2/5/0 = speed    p = pause",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "  m = bookmark   n/N = next/prev bookmark   / = search",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "  Enter = expand detail    Home/End = jump",
            Style::default().fg(Color::Yellow),
        )),
    ];
    let paragraph = Paragraph::new(help).alignment(Alignment::Center);
    frame.render_widget(paragraph, area);
}

/// Render the scrollable event list with cursor highlight.
fn render_event_list(frame: &mut Frame, session: &ReplaySession, area: Rect) {
    let block = Block::default()
        .title(" Events ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height as usize;
    if visible_height == 0 {
        return;
    }

    let (events, cursor_in_window) = session.visible_events(visible_height);

    let items: Vec<ListItem> = events
        .iter()
        .enumerate()
        .map(|(win_idx, (global_idx, ev))| {
            let is_cursor = win_idx == cursor_in_window;
            let is_bookmarked = session.is_bookmarked(*global_idx);

            let marker = if is_bookmarked { "*" } else { " " };
            let cat_color = cat_color(&ev.cat);

            let err_indicator = if ev.err.is_some() { " ERR" } else { "" };

            let line = Line::from(vec![
                Span::styled(
                    format!("{marker}{:>6} ", global_idx),
                    Style::default().fg(if is_bookmarked {
                        Color::LightYellow
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    format!("[{:<4}] ", ev.cat),
                    Style::default().fg(cat_color),
                ),
                Span::styled(
                    truncate(&ev.func, 30),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!(" {:.1}ms", ev.dur_ms),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    err_indicator.to_string(),
                    Style::default().fg(Color::Red),
                ),
            ]);

            if is_cursor {
                ListItem::new(line).style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::LightYellow),
                )
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Render the reconstructed state panel.
fn render_state_panel(frame: &mut Frame, session: &ReplaySession, area: Rect) {
    let block = Block::default()
        .title(" State at Cursor ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let state = session.state_at_cursor();

    // Build the state display lines
    let mut lines: Vec<Line> = Vec::new();

    // Current event detail
    if let Some(ev) = session.current_event() {
        lines.push(Line::from(Span::styled(
            " Current Event ",
            Style::default().fg(Color::LightCyan).bold(),
        )));
        lines.push(Line::from(vec![
            Span::styled("  fn: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&ev.func, Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  cat: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&ev.cat, Style::default().fg(cat_color(&ev.cat))),
            Span::styled(
                format!("  dur: {:.1}ms", ev.dur_ms),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        if ev.mem_growth != 0 {
            lines.push(Line::from(vec![
                Span::styled("  mem: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format_bytes(ev.mem_growth),
                    Style::default().fg(if ev.mem_growth > 0 {
                        Color::Yellow
                    } else {
                        Color::Green
                    }),
                ),
            ]));
        }

        if let Some(ref err) = ev.err {
            lines.push(Line::from(vec![
                Span::styled("  err: ", Style::default().fg(Color::Red)),
                Span::styled(
                    truncate(err, 40),
                    Style::default().fg(Color::Red),
                ),
            ]));
        }

        if session.detail_expanded {
            if let Some(ref preview) = ev.arg_preview {
                lines.push(Line::from(vec![
                    Span::styled("  args: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        truncate(preview, 40),
                        Style::default().fg(Color::White),
                    ),
                ]));
            }
            lines.push(Line::from(vec![
                Span::styled("  ts: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:.1}ms", ev.ts),
                    Style::default().fg(Color::White),
                ),
            ]));
        }

        lines.push(Line::from(""));
    }

    // Summary stats at cursor
    lines.push(Line::from(Span::styled(
        " Cumulative Stats ",
        Style::default().fg(Color::LightGreen).bold(),
    )));
    lines.push(Line::from(vec![
        Span::styled("  Events: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", state.events_processed),
            Style::default().fg(Color::White),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Mem growth: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_bytes(state.total_mem_growth),
            Style::default().fg(Color::White),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Errors: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", state.error_list.len()),
            Style::default().fg(if state.error_list.is_empty() {
                Color::Green
            } else {
                Color::Red
            }),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Renders: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", state.render_counts.len()),
            Style::default().fg(Color::White),
        ),
    ]));

    // Top render counts
    if !state.render_counts.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Top Renders ",
            Style::default().fg(Color::LightMagenta).bold(),
        )));

        let mut sorted: Vec<(&String, &u64)> = state.render_counts.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));

        for (name, count) in sorted.iter().take(5) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {}: ", truncate(name, 25)),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{count}"),
                    Style::default().fg(Color::White),
                ),
            ]));
        }
    }

    // Recent errors
    if !state.error_list.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Recent Errors ",
            Style::default().fg(Color::LightRed).bold(),
        )));

        for err in state.error_list.iter().rev().take(3) {
            lines.push(Line::from(Span::styled(
                format!("  {}", truncate(err, 45)),
                Style::default().fg(Color::Red),
            )));
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Render the replay-specific status bar.
fn render_replay_status_bar(
    frame: &mut Frame,
    session: &ReplaySession,
    area: Rect,
) {
    let cursor = session.cursor();
    let total = session.total_events();
    let pct = if total > 0 {
        (cursor as f64 / total as f64 * 100.0) as u16
    } else {
        0
    };

    let bookmarks = session.bookmarks();
    let bookmark_str = if bookmarks.is_empty() {
        String::new()
    } else {
        format!(" | {} bookmarks", bookmarks.len())
    };

    let filter_str = match session.filter() {
        Some(f) => format!(" | filter: {f}"),
        None => String::new(),
    };

    let status = format!(
        "{}/{total} ({pct}%) | {} | {}{}{}",
        cursor + 1,
        session.speed.label(),
        session.session_path,
        bookmark_str,
        filter_str,
    );

    let bar = Paragraph::new(status).style(
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray),
    );
    frame.render_widget(bar, area);
}

/// Get the display color for an event category.
fn cat_color(cat: &str) -> Color {
    match cat {
        "wasm" => Color::Cyan,
        "sql" => Color::Yellow,
        "net" => Color::Green,
        "mem" => Color::Magenta,
        "err" | "error" => Color::Red,
        "render" => Color::LightBlue,
        _ => Color::White,
    }
}

/// Truncate a string to max_len, adding "~" if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}~", &s[..max_len.saturating_sub(1)])
    } else {
        s.to_string()
    }
}
