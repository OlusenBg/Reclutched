//! Stage 1, step 1: connect to the Rocket League Stats API and print every raw message.

use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// How long to wait before retrying when the game isn't running yet.
const RETRY_DELAY: Duration = Duration::from_secs(2);

/// Connects to the Stats API at `addr` (e.g. "127.0.0.1:49123") and prints raw messages
/// until the process is stopped. If the game isn't up, keeps retrying quietly.
pub async fn run(addr: &str) -> Result<()> {
    let url = format!("ws://{addr}");
    loop {
        match connect_async(&url).await {
            Ok((mut ws, _response)) => {
                tracing::info!("connected to {url}");
                // `ws` is a Stream of messages; `.next().await` yields the next one,
                // or `None` when the connection closes.
                while let Some(msg) = ws.next().await {
                    match msg? {
                        Message::Text(text) => println!("{text}"),
                        Message::Close(_) => break,
                        _ => {} // ping/pong/binary: nothing to print
                    }
                }
                tracing::info!("connection closed, will retry");
            }
            Err(err) => tracing::debug!("cannot connect to {url} yet: {err}"),
        }
        tokio::time::sleep(RETRY_DELAY).await;
    }
}
