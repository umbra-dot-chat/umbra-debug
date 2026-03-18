//! Browser tab — displays browser-side vitals from the debug.ts heartbeat.
//!
//! Shows JS heap usage, DOM node count, render rate, message event throughput,
//! listener balance, and non-friend failure count. Events arrive with
//! `cat: "browser"` from the browser's debug.ts TUI bridge.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::App;
use super::format_bytes;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Browser Vitals ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let browser_events = app.events_by_cat("browser");

    if browser_events.is_empty() {
        let help = vec![
            Line::from(""),
            Line::from(Span::styled(
                "No browser events received yet.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "To connect, run in browser console:",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  __debug.connectTui(9999)",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "  __debug.autoConnectTui(9999)",
                Style::default().fg(Color::Yellow),
            )),
        ];
        let empty = Paragraph::new(help).alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let total = browser_events.len();
    let offset = if app.auto_scroll {
        total.saturating_sub(visible_height)
    } else {
        app.scroll_offset.min(total.saturating_sub(visible_height))
    };

    let visible = &browser_events[offset..total.min(offset + visible_height)];

    let items: Vec<ListItem> = visible
        .iter()
        .map(|ev| {
            let heap = format_bytes(ev.mem_after);
            let growth = format_bytes(ev.mem_growth);
            let dom = ev.arg_bytes; // We packed domNodes in argBytes

            // Color heap based on growth
            let heap_color = if ev.mem_growth > 5_242_880 {
                Color::Red // >5MB growth
            } else if ev.mem_growth > 1_048_576 {
                Color::Yellow // >1MB growth
            } else {
                Color::Green
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("#{:<4} ", ev.seq),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled("heap=", Style::default().fg(Color::DarkGray)),
                Span::styled(heap, Style::default().fg(heap_color)),
                Span::styled(
                    format!("({growth}) "),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("dom={dom} "),
                    Style::default().fg(if dom > 10000 {
                        Color::Red
                    } else if dom > 5000 {
                        Color::Yellow
                    } else {
                        Color::Cyan
                    }),
                ),
                Span::styled(
                    format!("{}", ev.func),
                    Style::default().fg(Color::Blue),
                ),
                if let Some(ref err) = ev.err {
                    Span::styled(
                        format!(" ERR: {}", &err[..err.len().min(60)]),
                        Style::default().fg(Color::Red),
                    )
                } else {
                    Span::raw("")
                },
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}
