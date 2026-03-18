//! Session replay — load a saved JSONL session and feed events
//! to the app at adjustable speed.
//!
//! Supports 1x, 5x, 10x, and real-time playback speeds.
//! All tabs are functional during replay.

use std::path::Path;

use color_eyre::eyre::Result;
use tokio::sync::mpsc;

use crate::server::WsEvent;
use crate::store;

/// Replay speed multiplier.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum Speed {
    RealTime,
    Fast5x,
    Fast10x,
}

#[allow(dead_code)]
impl Speed {
    /// Get the divisor for sleep duration.
    pub fn divisor(self) -> f64 {
        match self {
            Speed::RealTime => 1.0,
            Speed::Fast5x => 5.0,
            Speed::Fast10x => 10.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Speed::RealTime => "1x",
            Speed::Fast5x => "5x",
            Speed::Fast10x => "10x",
        }
    }

    pub fn next(self) -> Speed {
        match self {
            Speed::RealTime => Speed::Fast5x,
            Speed::Fast5x => Speed::Fast10x,
            Speed::Fast10x => Speed::RealTime,
        }
    }
}

/// Start replaying a session file, feeding events to the WsEvent channel.
///
/// Returns a handle that can be used to pause/resume/change speed.
#[allow(dead_code)]
pub async fn start_replay(
    path: &Path,
    tx: mpsc::UnboundedSender<WsEvent>,
) -> Result<ReplayHandle> {
    let events = store::load_session(path)?;
    let total = events.len();

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<ReplayCommand>();

    tokio::spawn(async move {
        let mut paused = false;
        let mut speed = Speed::RealTime;
        let mut idx = 0;

        while idx < events.len() {
            // Check for commands (non-blocking)
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    ReplayCommand::Pause => paused = true,
                    ReplayCommand::Resume => paused = false,
                    ReplayCommand::TogglePause => paused = !paused,
                    ReplayCommand::SetSpeed(s) => speed = s,
                    ReplayCommand::Stop => return,
                }
            }

            if paused {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }

            let event = &events[idx];

            // Calculate sleep duration based on timestamp delta
            if idx > 0 {
                let delta_ms = event.ts - events[idx - 1].ts;
                if delta_ms > 0.0 {
                    let sleep_ms = (delta_ms / speed.divisor()).max(1.0);
                    // Cap at 2 seconds to avoid long pauses
                    let sleep_ms = sleep_ms.min(2000.0);
                    tokio::time::sleep(std::time::Duration::from_millis(
                        sleep_ms as u64,
                    ))
                    .await;
                }
            }

            let _ = tx.send(WsEvent::Trace(event.clone()));
            idx += 1;
        }
    });

    Ok(ReplayHandle {
        cmd_tx,
        total_events: total,
    })
}

/// Commands sent to the replay task.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ReplayCommand {
    Pause,
    Resume,
    TogglePause,
    SetSpeed(Speed),
    Stop,
}

/// Handle for controlling a running replay.
#[allow(dead_code)]
pub struct ReplayHandle {
    cmd_tx: mpsc::UnboundedSender<ReplayCommand>,
    pub total_events: usize,
}

#[allow(dead_code)]
impl ReplayHandle {
    pub fn toggle_pause(&self) {
        let _ = self.cmd_tx.send(ReplayCommand::TogglePause);
    }

    pub fn set_speed(&self, speed: Speed) {
        let _ = self.cmd_tx.send(ReplayCommand::SetSpeed(speed));
    }

    pub fn stop(&self) {
        let _ = self.cmd_tx.send(ReplayCommand::Stop);
    }
}
