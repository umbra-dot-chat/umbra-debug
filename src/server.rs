//! WebSocket server for receiving trace events from browser clients.
//!
//! Accepts multiple concurrent connections on a configurable port.
//! Each client sends an initial "hello" handshake, then streams
//! TraceEvent JSON messages. Events are forwarded to the app state
//! via an mpsc channel.

use std::net::SocketAddr;

use color_eyre::eyre::Result;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::app::TraceEvent;

/// Message types received from browser clients.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ClientMessage {
    /// Initial handshake from a connecting client.
    Hello {
        #[serde(rename = "clientId")]
        client_id: String,
        #[serde(rename = "userAgent", default)]
        user_agent: String,
        #[serde(rename = "deviceMemory", default)]
        device_memory: f64,
    },
    /// A trace event from the instrumented WASM layer.
    Trace(TraceEvent),
}

/// Events sent from the WebSocket server to the app.
#[derive(Debug, Clone)]
pub enum WsEvent {
    /// A new client connected.
    ClientConnected {
        client_id: String,
        user_agent: String,
        device_memory: f64,
    },
    /// A trace event was received.
    Trace(TraceEvent),
    /// A client disconnected (possibly unexpectedly).
    ClientDisconnected {
        client_id: String,
        clean: bool,
    },
}

/// Start the WebSocket server on the given port.
///
/// Spawns a background task that accepts connections and forwards
/// events to the provided channel.
pub async fn start(port: u16, tx: mpsc::UnboundedSender<WsEvent>) -> Result<()> {
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let listener = TcpListener::bind(addr).await?;

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let tx = tx.clone();
                    tokio::spawn(handle_connection(stream, peer, tx));
                }
                Err(e) => {
                    eprintln!("Accept error: {e}");
                }
            }
        }
    });

    Ok(())
}

/// Handle a single WebSocket connection.
async fn handle_connection(
    stream: TcpStream,
    _peer: SocketAddr,
    tx: mpsc::UnboundedSender<WsEvent>,
) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("WebSocket handshake failed: {e}");
            return;
        }
    };

    let (mut _write, mut read) = ws_stream.split();
    let mut client_id = String::from("unknown");
    let mut clean_disconnect = false;

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                // Try parsing as a hello message first
                if let Ok(ClientMessage::Hello {
                    client_id: cid,
                    user_agent,
                    device_memory,
                }) = serde_json::from_str::<ClientMessage>(&text)
                {
                    client_id = cid.clone();
                    let _ = tx.send(WsEvent::ClientConnected {
                        client_id: cid,
                        user_agent,
                        device_memory,
                    });
                    continue;
                }

                // Otherwise parse as a trace event
                match serde_json::from_str::<TraceEvent>(&text) {
                    Ok(event) => {
                        let _ = tx.send(WsEvent::Trace(event));
                    }
                    Err(e) => {
                        eprintln!("Failed to parse trace event: {e}");
                    }
                }
            }
            Ok(Message::Close(_)) => {
                clean_disconnect = true;
                break;
            }
            Ok(_) => {} // Ignore binary, ping, pong
            Err(e) => {
                eprintln!("WebSocket read error: {e}");
                break;
            }
        }
    }

    let _ = tx.send(WsEvent::ClientDisconnected {
        client_id,
        clean: clean_disconnect,
    });
}
