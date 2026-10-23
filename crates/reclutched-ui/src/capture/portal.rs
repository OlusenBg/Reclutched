//! The desktop's screen-share portal (xdg-desktop-portal). We ask once what to capture (the
//! desktop shows its own picker), remember the answer, and then hand a fresh PipeWire
//! connection to every capture, because one connection cannot be shared by two programs.

use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, StartCastOptions,
};
use ashpd::desktop::PersistMode;
use enumflags2::BitFlags;

/// What the portal is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Off,
    /// Waiting for the desktop's picker (or the saved choice).
    Asking,
    /// Capturing this stream.
    Ready {
        node: u32,
        size: Option<(i32, i32)>,
    },
    Failed(String),
}

enum Command {
    Begin,
    End,
    Forget,
    Remote(mpsc::Sender<Result<OwnedFd, String>>),
}

pub struct Portal {
    tx: tokio::sync::mpsc::UnboundedSender<Command>,
    state: Arc<Mutex<State>>,
}

impl Portal {
    /// Starts the portal thread. `token_path` is where your choice (screen or window) is kept.
    pub fn start(token_path: PathBuf) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(State::Off));
        let shared = state.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(rt) => rt.block_on(serve(rx, shared, token_path)),
                Err(e) => set(&shared, State::Failed(e.to_string())),
            }
        });
        Self { tx, state }
    }

    /// Asks the desktop what to capture (or reuses the saved answer) and starts the session.
    pub fn begin(&self) {
        set(&self.state, State::Asking);
        let _ = self.tx.send(Command::Begin);
    }

    pub fn end(&self) {
        let _ = self.tx.send(Command::End);
    }

    /// Forgets what you chose, so the next `begin` shows the picker again.
    pub fn forget_choice(&self) {
        let _ = self.tx.send(Command::Forget);
    }

    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or(State::Off)
    }

    /// A new PipeWire connection for one capture (blocks for a moment).
    pub fn remote(&self) -> Result<OwnedFd, String> {
        let (reply, answer) = mpsc::channel();
        self.tx
            .send(Command::Remote(reply))
            .map_err(|_| "the screen capture service stopped".to_string())?;
        answer
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "the desktop did not answer in time".to_string())?
    }
}

fn set(state: &Mutex<State>, value: State) {
    if let Ok(mut s) = state.lock() {
        *s = value;
    }
}

type Session = ashpd::desktop::Session<Screencast>;

async fn serve(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
    state: Arc<Mutex<State>>,
    token_path: PathBuf,
) {
    let mut open: Option<(Screencast, Session)> = None;
    while let Some(command) = rx.recv().await {
        match command {
            Command::Begin => {
                if let Some((_, old)) = open.take() {
                    let _ = old.close().await;
                }
                match begin(&token_path).await {
                    Ok((proxy, session, node, size)) => {
                        set(&state, State::Ready { node, size });
                        open = Some((proxy, session));
                    }
                    Err(e) => set(&state, State::Failed(e)),
                }
            }
            Command::End => {
                if let Some((_, session)) = open.take() {
                    let _ = session.close().await;
                }
                set(&state, State::Off);
            }
            Command::Forget => {
                let _ = std::fs::remove_file(&token_path);
            }
            Command::Remote(reply) => {
                let result = match &open {
                    Some((proxy, session)) => proxy
                        .open_pipe_wire_remote(session, Default::default())
                        .await
                        .map_err(|e| e.to_string()),
                    None => Err("capturing is not on".to_string()),
                };
                let _ = reply.send(result);
            }
        }
    }
}

async fn begin(
    token_path: &PathBuf,
) -> Result<(Screencast, Session, u32, Option<(i32, i32)>), String> {
    let proxy = Screencast::new()
        .await
        .map_err(|e| format!("the desktop has no screen capture portal ({e})"))?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|e| e.to_string())?;
    let token = std::fs::read_to_string(token_path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let sources: BitFlags<SourceType> = SourceType::Monitor | SourceType::Window;
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Embedded)
                .set_sources(sources)
                .set_multiple(false)
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(token.as_deref()),
        )
        .await
        .map_err(|e| e.to_string())?;
    let response = proxy
        .start(&session, None, StartCastOptions::default())
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| format!("capturing was not allowed ({e})"))?;
    if let Some(new_token) = response.restore_token() {
        if let Some(dir) = token_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(token_path, new_token);
    }
    let stream = response
        .streams()
        .first()
        .ok_or_else(|| "nothing was chosen to capture".to_string())?;
    Ok((proxy, session, stream.pipe_wire_node_id(), stream.size()))
}
