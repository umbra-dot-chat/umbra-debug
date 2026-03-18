//! # Umbra Debug TUI
//!
//! Terminal inspector for Umbra WASM trace events. Receives trace
//! data via WebSocket from the browser and displays it in an
//! interactive ratatui-based terminal UI with multiple analysis tabs.
//!
//! ## Usage
//!
//! ```bash
//! # Launch TUI mode (default)
//! umbra-debug
//!
//! # Launch on a custom port
//! umbra-debug --port 8888
//!
//! # Replay a saved session
//! umbra-debug --replay ~/.umbra/debug-logs/session-2026-03-12T120000.jsonl
//!
//! # CLI query mode
//! umbra-debug query --last-crash
//! umbra-debug query --memory-suspects
//! umbra-debug query --hot-functions
//! umbra-debug query --grep "store_incoming"
//! umbra-debug query --tail
//! umbra-debug query --memory-timeline
//! umbra-debug query --slow-wasm 50
//! umbra-debug query --render-storms
//! umbra-debug query --state-changes AuthContext
//! umbra-debug query --timeline 12:00 12:05
//! umbra-debug query --errors-only
//! umbra-debug query --budget-violations
//!
//! # Compare two sessions side-by-side
//! umbra-debug --compare session1.jsonl session2.jsonl
//! ```

mod app;
mod crash_report;
mod replay;
mod server;
mod store;
mod tui;
mod ui;

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;

/// Umbra Debug TUI — trace event inspector for WASM debugging.
#[derive(Parser)]
#[command(name = "umbra-debug", version, about)]
struct Cli {
    /// WebSocket server port (default: 9999)
    #[arg(long, default_value_t = 9999)]
    port: u16,

    /// Replay a saved JSONL session file
    #[arg(long, value_name = "FILE")]
    replay: Option<String>,

    /// Enable verbose logging
    #[arg(long)]
    verbose: bool,

    /// Compare two session files side-by-side
    #[arg(long, num_args = 2, value_names = ["SESSION_A", "SESSION_B"])]
    compare: Option<Vec<String>>,

    /// CLI query subcommand (skip TUI)
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Query saved sessions without launching the TUI
    Query {
        /// Print the last crash report
        #[arg(long)]
        last_crash: bool,

        /// Print top 20 memory-growing functions from last session
        #[arg(long)]
        memory_suspects: bool,

        /// Print top 20 functions by call count from last session
        #[arg(long)]
        hot_functions: bool,

        /// Regex search across all session files
        #[arg(long, value_name = "PATTERN")]
        grep: Option<String>,

        /// Tail live events from a running TUI instance
        #[arg(long)]
        tail: bool,

        /// Print ASCII memory timeline from last session
        #[arg(long)]
        memory_timeline: bool,

        /// Print WASM calls exceeding threshold in ms
        #[arg(long, value_name = "MS")]
        slow_wasm: Option<f64>,

        /// Print components with render storms (>100/s)
        #[arg(long)]
        render_storms: bool,

        /// Print state changes matching a context name
        #[arg(long, value_name = "CONTEXT")]
        state_changes: Option<String>,

        /// Print events within a time range (two HH:MM args)
        #[arg(long, num_args = 2, value_names = ["START", "END"])]
        timeline: Option<Vec<String>>,

        /// Print only error and fatal events
        #[arg(long)]
        errors_only: bool,

        /// Print budget violation events
        #[arg(long)]
        budget_violations: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    // CLI query mode — print to stdout and exit
    if let Some(Commands::Query {
        last_crash,
        memory_suspects,
        hot_functions,
        grep,
        tail,
        memory_timeline,
        slow_wasm,
        render_storms,
        state_changes,
        timeline,
        errors_only,
        budget_violations,
    }) = cli.command
    {
        return run_query(
            last_crash,
            memory_suspects,
            hot_functions,
            grep,
            tail,
            memory_timeline,
            slow_wasm,
            render_storms,
            state_changes,
            timeline,
            errors_only,
            budget_violations,
        )
        .await;
    }

    // TUI mode (default or --replay or --compare)
    run_tui(cli.port, cli.replay, cli.compare, cli.verbose).await
}

/// Run the interactive TUI.
async fn run_tui(
    port: u16,
    replay_path: Option<String>,
    compare_paths: Option<Vec<String>>,
    _verbose: bool,
) -> Result<()> {
    use app::App;
    use crossterm::event::{self, Event, KeyEventKind};
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::sync::mpsc;

    // Channel for trace events from WebSocket server
    let (ws_tx, mut ws_rx) = mpsc::unbounded_channel();

    let mut app = App::new();

    // If --compare was given, load the comparison and switch to Compare tab
    if let Some(paths) = compare_paths {
        if paths.len() == 2 {
            let path_a = PathBuf::from(&paths[0]);
            let path_b = PathBuf::from(&paths[1]);
            app.load_comparison_from_files(&path_a, &path_b);
            app.tab = app::Tab::Compare;
        }
    }

    // Start WebSocket server (unless in replay mode)
    if replay_path.is_none() {
        server::start(port, ws_tx.clone()).await?;
    }

    // Initialize terminal
    let mut terminal = tui::init()?;

    // Tick interval for UI refresh
    let mut tick_interval = tokio::time::interval(Duration::from_millis(200));

    loop {
        // Render
        terminal.draw(|frame| app.render(frame))?;

        if app.should_quit {
            break;
        }

        // Always check for terminal key events first (non-blocking)
        while event::poll(Duration::ZERO)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key.code, key.modifiers);
                }
            }
        }

        if app.should_quit {
            break;
        }

        // Wait for either: WS events (drain up to 100 per cycle) or tick
        tokio::select! {
            _ = tick_interval.tick() => {
                app.tick();
            }
            msg = ws_rx.recv() => {
                if let Some(evt) = msg {
                    app.handle_ws_event(evt);
                }
                // Drain buffered WS events (up to 100 to avoid starving render)
                for _ in 0..100 {
                    match ws_rx.try_recv() {
                        Ok(evt) => app.handle_ws_event(evt),
                        Err(_) => break,
                    }
                }
            }
        }
    }

    // Restore terminal
    tui::restore()?;
    Ok(())
}

/// Run CLI query mode — prints to stdout and exits.
#[allow(clippy::too_many_arguments)]
async fn run_query(
    last_crash: bool,
    memory_suspects: bool,
    hot_functions: bool,
    grep: Option<String>,
    _tail: bool,
    memory_timeline: bool,
    slow_wasm: Option<f64>,
    render_storms: bool,
    state_changes: Option<String>,
    timeline: Option<Vec<String>>,
    errors_only: bool,
    budget_violations: bool,
) -> Result<()> {
    let log_dir = store::log_dir();

    if last_crash {
        store::query::print_last_crash(&log_dir)?;
    } else if memory_suspects {
        store::query::print_memory_suspects(&log_dir)?;
    } else if hot_functions {
        store::query::print_hot_functions(&log_dir)?;
    } else if let Some(pattern) = grep {
        store::query::print_grep(&log_dir, &pattern)?;
    } else if memory_timeline {
        store::query::print_memory_timeline(&log_dir)?;
    } else if let Some(threshold) = slow_wasm {
        store::query::print_slow_wasm(&log_dir, threshold)?;
    } else if render_storms {
        store::query::print_render_storms(&log_dir)?;
    } else if let Some(context) = state_changes {
        store::query::print_state_changes(&log_dir, &context)?;
    } else if let Some(range) = timeline {
        if range.len() == 2 {
            store::query::print_timeline(&log_dir, &range[0], &range[1])?;
        }
    } else if errors_only {
        store::query::print_errors_only(&log_dir)?;
    } else if budget_violations {
        store::query::print_budget_violations(&log_dir)?;
    } else {
        println!("No query flag provided. Use --help for options.");
    }

    Ok(())
}
