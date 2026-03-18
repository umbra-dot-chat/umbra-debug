//! Terminal initialization, restoration, and panic handling.
//!
//! Manages raw mode, alternate screen, and ensures the terminal
//! is always restored even on panic or unexpected exit.

use std::io::{self, stdout, Stdout};

use color_eyre::eyre::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

/// Type alias for our terminal backend.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Initialize the terminal for TUI rendering.
///
/// Enables raw mode, enters alternate screen, and returns a
/// configured terminal instance. Must be paired with [`restore`]
/// on exit.
pub fn init() -> Result<Tui> {
    install_panic_hook();
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to its original state.
///
/// Disables raw mode and leaves alternate screen. Should be called
/// before the process exits to prevent a stuck terminal.
pub fn restore() -> Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Install a panic hook that restores the terminal before printing
/// the panic message. Without this, a panic would leave the terminal
/// in raw mode with the alternate screen active.
fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Best-effort terminal restore — ignore errors since we're panicking
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));
}
