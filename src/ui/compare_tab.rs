//! Compare tab — side-by-side diff of two debug sessions.
//!
//! Left column shows Session A metrics, right column shows
//! Session B metrics, and the bottom highlights regressions
//! (red) and improvements (green).

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::store::compare::SessionComparison;
use super::format_bytes;

/// Render the Compare tab.
///
/// If `comparison` is `None`, shows a prompt to select sessions.
pub fn render(
    frame: &mut Frame,
    comparison: Option<&SessionComparison>,
    area: Rect,
) {
    let block = Block::default()
        .title(" Compare (C to select sessions) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightCyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let comp = match comparison {
        Some(c) => c,
        None => {
            let hint = Paragraph::new(
                "Press C to select two sessions for comparison.",
            )
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
            frame.render_widget(hint, inner);
            return;
        }
    };

    // Layout: summary | memory diff | event freq | regressions
    let sections = Layout::vertical([
        Constraint::Length(6), // Summary side-by-side
        Constraint::Length(1), // Spacer
        Constraint::Length(1), // Event freq header
        Constraint::Length(8), // Event freq table
        Constraint::Length(1), // Regressions header
        Constraint::Min(4),   // Regressions / new errors
    ])
    .split(inner);

    render_summary(frame, comp, sections[0]);
    render_event_freq(frame, comp, sections[2], sections[3]);
    render_regressions(frame, comp, sections[4], sections[5]);
}

/// Two-column summary of session A vs session B.
fn render_summary(frame: &mut Frame, comp: &SessionComparison, area: Rect) {
    let cols = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ])
    .split(area);

    // Session A summary (left)
    let a = &comp.session_a;
    let a_name = a
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let a_lines = vec![
        Line::from(Span::styled(
            format!(" A: {a_name}"),
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(format!(
            "   Duration:  {:.1}s",
            a.duration_secs
        )),
        Line::from(format!("   Events:    {}", a.total_events)),
        Line::from(format!("   Errors:    {}", a.total_errors)),
        Line::from(format!(
            "   Max Mem:   {}",
            format_bytes(a.max_memory)
        )),
        Line::from(format!(
            "   Render/s:  {:.1}",
            a.avg_render_rate
        )),
    ];
    let a_para = Paragraph::new(a_lines);
    frame.render_widget(a_para, cols[0]);

    // Session B summary (right)
    let b = &comp.session_b;
    let b_name = b
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();

    let mem_color = diff_color(a.max_memory, b.max_memory, true);
    let err_color = diff_color(
        a.total_errors as i64,
        b.total_errors as i64,
        true,
    );
    let render_color = diff_color_f64(
        a.avg_render_rate,
        b.avg_render_rate,
        true,
    );

    let b_lines = vec![
        Line::from(Span::styled(
            format!(" B: {b_name}"),
            Style::default().fg(Color::LightMagenta).bold(),
        )),
        Line::from(format!(
            "   Duration:  {:.1}s",
            b.duration_secs
        )),
        Line::from(format!("   Events:    {}", b.total_events)),
        Line::from(vec![
            Span::raw("   Errors:    "),
            Span::styled(
                format!("{}", b.total_errors),
                Style::default().fg(err_color),
            ),
            delta_span(a.total_errors as i64, b.total_errors as i64),
        ]),
        Line::from(vec![
            Span::raw("   Max Mem:   "),
            Span::styled(
                format_bytes(b.max_memory),
                Style::default().fg(mem_color),
            ),
        ]),
        Line::from(vec![
            Span::raw("   Render/s:  "),
            Span::styled(
                format!("{:.1}", b.avg_render_rate),
                Style::default().fg(render_color),
            ),
        ]),
    ];
    let b_para = Paragraph::new(b_lines);
    frame.render_widget(b_para, cols[1]);
}

/// Event frequency diff table.
fn render_event_freq(
    frame: &mut Frame,
    comp: &SessionComparison,
    header_area: Rect,
    table_area: Rect,
) {
    let header = Paragraph::new(Span::styled(
        " Event Frequency by Category ",
        Style::default().fg(Color::LightBlue).bold(),
    ));
    frame.render_widget(header, header_area);

    let mut cats: Vec<(&String, &(u64, u64))> =
        comp.event_freq_diff.iter().collect();
    cats.sort_by(|a, b| {
        (b.1 .0 + b.1 .1).cmp(&(a.1 .0 + a.1 .1))
    });

    let rows: Vec<Row> = cats
        .iter()
        .take(8)
        .map(|(cat, (a_count, b_count))| {
            let color = diff_color(*a_count as i64, *b_count as i64, true);
            Row::new(vec![
                Cell::from(cat.to_string()),
                Cell::from(format!("{a_count}")),
                Cell::from(format!("{b_count}"))
                    .style(Style::default().fg(color)),
                Cell::from(delta_string(*a_count as i64, *b_count as i64))
                    .style(Style::default().fg(color)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(12),
    ];
    let header_row = Row::new(vec![
        Cell::from("Category")
            .style(Style::default().fg(Color::DarkGray)),
        Cell::from("A")
            .style(Style::default().fg(Color::Cyan)),
        Cell::from("B")
            .style(Style::default().fg(Color::LightMagenta)),
        Cell::from("Delta")
            .style(Style::default().fg(Color::DarkGray)),
    ]);
    let table = Table::new(rows, widths).header(header_row);
    frame.render_widget(table, table_area);
}

/// WASM regressions and new errors.
fn render_regressions(
    frame: &mut Frame,
    comp: &SessionComparison,
    header_area: Rect,
    content_area: Rect,
) {
    let header = Paragraph::new(Span::styled(
        " Regressions & New Errors ",
        Style::default().fg(Color::LightRed).bold(),
    ));
    frame.render_widget(header, header_area);

    let mut lines: Vec<Line> = Vec::new();

    // WASM regressions (top 5)
    if !comp.wasm_regressions.is_empty() {
        lines.push(Line::from(Span::styled(
            "  WASM Timing Regressions:",
            Style::default().fg(Color::Red),
        )));
        for (func, a_ms, b_ms) in comp.wasm_regressions.iter().take(5) {
            let pct = if *a_ms > 0.0 {
                (b_ms / a_ms - 1.0) * 100.0
            } else {
                0.0
            };
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("{func}: "),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{a_ms:.1}ms"),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(" -> ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{b_ms:.1}ms"),
                    Style::default().fg(Color::Red),
                ),
                Span::styled(
                    format!(" (+{pct:.0}%)"),
                    Style::default().fg(Color::Red),
                ),
            ]));
        }
    }

    // New errors
    if !comp.new_errors.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            format!(
                "  New Errors in B ({} unique):",
                comp.new_errors.len()
            ),
            Style::default().fg(Color::Red),
        )));
        for err in comp.new_errors.iter().take(5) {
            let truncated = if err.len() > 80 {
                format!("{}...", &err[..77])
            } else {
                err.clone()
            };
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(truncated, Style::default().fg(Color::LightRed)),
            ]));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No regressions or new errors detected.",
            Style::default().fg(Color::Green),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, content_area);
}

// -- Helpers --

/// Color for a metric where higher is worse.
fn diff_color(a: i64, b: i64, higher_is_worse: bool) -> Color {
    if b == a {
        Color::White
    } else if (b > a) == higher_is_worse {
        Color::Red
    } else {
        Color::Green
    }
}

/// Color for float metrics.
fn diff_color_f64(a: f64, b: f64, higher_is_worse: bool) -> Color {
    let threshold = a * 0.05; // 5% change threshold
    if (b - a).abs() < threshold {
        Color::White
    } else if (b > a) == higher_is_worse {
        Color::Red
    } else {
        Color::Green
    }
}

/// Produce a "+N" / "-N" delta string.
fn delta_string(a: i64, b: i64) -> String {
    let diff = b - a;
    if diff > 0 {
        format!("+{diff}")
    } else if diff < 0 {
        format!("{diff}")
    } else {
        "=".to_string()
    }
}

/// Produce a styled " (+N)" or " (-N)" span.
fn delta_span(a: i64, b: i64) -> Span<'static> {
    let diff = b - a;
    if diff > 0 {
        Span::styled(
            format!(" (+{diff})"),
            Style::default().fg(Color::Red),
        )
    } else if diff < 0 {
        Span::styled(
            format!(" ({diff})"),
            Style::default().fg(Color::Green),
        )
    } else {
        Span::styled(" (=)", Style::default().fg(Color::DarkGray))
    }
}
