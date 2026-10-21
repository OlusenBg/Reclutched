//! Connects to the Stats API websocket and hands out raw messages.

use std::sync::Arc;
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
    spawn_inner(addr, None)
}

/// Like [`spawn`], and also calls `status(true)` when the game connects and `status(false)` when
/// it is not reachable (the first time we fail, and again after a connection was lost).
pub fn spawn_with_status(
    addr: &str,
    status: impl Fn(bool) + Send + Sync + 'static,
) -> mpsc::UnboundedReceiver<String> {
    spawn_inner(addr, Some(Arc::new(status)))
}

type Status = Arc<dyn Fn(bool) + Send + Sync>;

fn spawn_inner(addr: &str, status: Option<Status>) -> mpsc::UnboundedReceiver<String> {
    let url = format!("ws://{addr}");
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        // Only report changes, not every failed retry.
        let mut last: Option<bool> = None;
        let mut report = |connected: bool| {
            if last != Some(connected) {
                last = Some(connected);
                if let Some(f) = &status {
                    f(connected);
                }
            }
        };
        loop {
            match connect_async(&url).await {
                Ok((mut ws, _response)) => {
                    tracing::info!("connected to {url}");
                    report(true);
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
                    report(false);
                }
                Err(err) => {
                    tracing::debug!("cannot connect to {url} yet: {err}");
                    report(false);
                }
            }
            if tx.is_closed() {
                return;
            }
            tokio::time::sleep(RETRY_DELAY).await;
        }
    });
    rx
}
