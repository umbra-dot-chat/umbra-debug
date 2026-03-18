//! UI rendering dispatch and status bar.
//!
//! Routes rendering to the active tab module and draws the
//! persistent tab bar and status bar.

pub mod all_tab;
pub mod analysis_tab;
pub mod breakpoint;
pub mod browser_tab;
pub mod compare_tab;
pub mod dashboard_tab;
pub mod deps_tab;
pub mod err_tab;
pub mod log_tab;
pub mod mem_tab;
pub mod net_tab;
pub mod replay_tab;
pub mod sql_tab;
pub mod wasm_tab;

use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Tabs};

use crate::app::{App, Tab};

/// Render the full UI: tab bar + main content + status bar.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Vertical layout: tab bar | content | filter bar? | status bar
    let has_filter = app.filter_mode || app.log_source_input_mode || app.replay_filter_mode;
    let constraints = if has_filter {
        vec![
            Constraint::Length(1), // Tab bar
            Constraint::Min(5),   // Content
            Constraint::Length(1), // Filter input
            Constraint::Length(1), // Status bar
        ]
    } else {
        vec![
            Constraint::Length(1), // Tab bar
            Constraint::Min(5),   // Content
            Constraint::Length(1), // Status bar
        ]
    };

    let chunks = Layout::vertical(constraints).split(area);

    // 1. Tab bar
    render_tab_bar(frame, app.tab, chunks[0]);

    // 2. Main content area — dispatch to active tab
    let content_area = chunks[1];
    match app.tab {
        Tab::Dashboard => dashboard_tab::render(frame, app, content_area),
        Tab::All => all_tab::render(frame, app, content_area),
        Tab::Wasm => wasm_tab::render(frame, app, content_area),
        Tab::Sql => sql_tab::render(frame, app, content_area),
        Tab::Net => net_tab::render(frame, app, content_area),
        Tab::Mem => mem_tab::render(frame, app, content_area),
        Tab::Err => err_tab::render(frame, app, content_area),
        Tab::Analysis => analysis_tab::render(frame, app, content_area),
        Tab::Browser => browser_tab::render(frame, app, content_area),
        Tab::Log => log_tab::render(frame, app, content_area),
        Tab::Compare => compare_tab::render(
            frame,
            app.comparison.as_ref(),
            content_area,
        ),
        Tab::Replay => replay_tab::render(frame, app, content_area),
        Tab::Deps => deps_tab::render(frame, app, content_area),
    }

    // 2b. Breakpoint input dialog overlay (renders on top of content)
    if app.bp_input_mode {
        breakpoint::render_breakpoint_dialog(frame, &app.bp_input, content_area);
    }

    // 3. Filter input bar (if active)
    if app.filter_mode {
        render_filter_bar(frame, &app.filter_input, chunks[2]);
    } else if app.log_source_input_mode {
        render_filter_bar(frame, &format!("src:{}", app.log_source_input), chunks[2]);
    } else if app.replay_filter_mode {
        render_filter_bar(frame, &format!("replay:{}", app.replay_filter_input), chunks[2]);
    }

    // 4. Status bar (breakpoint pause overrides normal status)
    let status_area = if has_filter { chunks[3] } else { chunks[2] };
    if app.bp_paused {
        let reason = app.bp_pause_reason.as_deref().unwrap_or("Breakpoint");
        breakpoint::render_pause_indicator(frame, reason, status_area);
    } else {
        render_status_bar(frame, app, status_area);
    }
}

/// Render the tab bar at the top.
fn render_tab_bar(frame: &mut Frame, active: Tab, area: Rect) {
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .map(|t| {
            if *t == active {
                Line::from(Span::styled(
                    format!(" {} ", t.label()),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .bold(),
                ))
            } else {
                Line::from(Span::styled(
                    format!(" {} ", t.label()),
                    Style::default().fg(Color::DarkGray),
                ))
            }
        })
        .collect();

    let active_idx = Tab::ALL.iter().position(|&t| t == active).unwrap_or(0);
    let tabs = Tabs::new(titles)
        .select(active_idx)
        .divider(Span::styled("|", Style::default().fg(Color::DarkGray)));

    frame.render_widget(tabs, area);
}

/// Render the filter input bar.
fn render_filter_bar(frame: &mut Frame, input: &str, area: Rect) {
    let text = format!("/{input}_");
    let bar = Paragraph::new(text).style(
        Style::default()
            .fg(Color::Yellow)
            .bg(Color::DarkGray),
    );
    frame.render_widget(bar, area);
}

/// Render the status bar at the bottom.
fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let client_count = app.clients.len();
    let conn_status = if client_count > 0 {
        format!("Connected ({client_count} client{})", if client_count == 1 { "" } else { "s" })
    } else {
        "Waiting for connection...".to_string()
    };

    let wasm_mem = format_bytes(app.latest_mem());
    let total_growth = format_bytes(app.total_mem_growth());

    let mut parts = vec![
        conn_status,
        format!("{:.0} ev/s", app.events_per_sec),
        format!("WASM: {wasm_mem}"),
        format!("Growth: {total_growth}"),
        app.session_duration(),
    ];

    if app.paused {
        parts.push("[PAUSED]".to_string());
    }

    if app.filter.is_some() {
        parts.push(format!("[filter: {}]", app.filter_input));
    }

    if !app.breakpoints.is_empty() {
        let enabled = app.breakpoints.iter().filter(|b| b.enabled).count();
        parts.push(format!("[bp: {enabled}]"));
    }

    let status_text = parts.join(" | ");
    let bar = Paragraph::new(status_text).style(
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray),
    );
    frame.render_widget(bar, area);
}

/// Format bytes as a compact human-readable string.
pub fn format_bytes(bytes: i64) -> String {
    let abs = bytes.unsigned_abs();
    let sign = if bytes < 0 { "-" } else { "" };
    if abs >= 1_073_741_824 {
        format!("{sign}{:.1}GB", abs as f64 / 1_073_741_824.0)
    } else if abs >= 1_048_576 {
        format!("{sign}{:.1}MB", abs as f64 / 1_048_576.0)
    } else if abs >= 1024 {
        format!("{sign}{:.1}KB", abs as f64 / 1024.0)
    } else {
        format!("{sign}{abs}B")
    }
}
