//! Connects to the Stats API websocket and hands out raw messages.

use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// How long to wait before retrying when the game isn't running yet.
const RETRY_DELAY: Duration = Duration::from_secs(2);

/// Starts a background task that connects to `addr` (e.g. "127.0.0.1:49124"), keeps
/// reconnecting, and sends every text message into the returned channel.
///
/// `tokio::spawn` runs the task on its own; it stops by itself once the receiver is dropped.
pub fn spawn(addr: &str) -> mpsc::UnboundedReceiver<String> {
    let url = format!("ws://{addr}");
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            match connect_async(&url).await {
                Ok((mut ws, _response)) => {
                    tracing::info!("connected to {url}");
                    // `ws` is a Stream of messages; `.next().await` yields the next one,
                    // or `None` when the connection closes.
                    while let Some(msg) = ws.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                // Err means the receiver is gone: nobody is listening any more.
                                if tx.send(text.to_string()).is_err() {
                                    return;
                                }
                            }
                            Ok(Message::Close(_)) | Err(_) => break,
                            Ok(_) => {} // ping/pong/binary: nothing to forward
                        }
                    }
                    tracing::info!("connection closed, will retry");
                }
                Err(err) => tracing::debug!("cannot connect to {url} yet: {err}"),
            }
            tokio::time::sleep(RETRY_DELAY).await;
        }
    });
    rx
}
