use clap::{Parser, Subcommand};

/// Linux-native game stats tracker.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print raw Rocket League Stats API events to the terminal.
    Dump {
        /// Address of the Stats API websocket.
        #[arg(long, default_value = "127.0.0.1:49123")]
        addr: String,
    },
}

// `#[tokio::main]` turns this async fn into a normal `main` that starts the async runtime.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Log level comes from RUST_LOG (e.g. RUST_LOG=debug); defaults to info.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr) // keep stdout clean for the raw dump
        .init();

    match Cli::parse().command {
        Command::Dump { addr } => {
            // Run until Ctrl-C; `select!` returns as soon as either branch finishes.
            tokio::select! {
                res = reclutched_daemon::dump::run(&addr) => res,
                _ = tokio::signal::ctrl_c() => Ok(()),
            }
        }
    }
}
