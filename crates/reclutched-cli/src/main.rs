use clap::{Parser, Subcommand};
use reclutched_core::rocketleague::settings::{Settings, TOGGLES};

/// Linux-native game stats tracker.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show every switch and whether it is on.
    List,
    /// Turn a switch on.
    Enable { name: String },
    /// Turn a switch off.
    Disable { name: String },
}

#[derive(Subcommand)]
enum Command {
    /// Choose what Reclutched collects and stores.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
        /// Settings file (default: ~/.config/reclutched/settings.json).
        #[arg(long)]
        file: Option<std::path::PathBuf>,
    },
    /// Show your saved matches (green = win, red = loss).
    History {
        /// How many matches to show.
        #[arg(long, default_value_t = 20)]
        limit: u32,
        /// Database file (default: ~/.local/share/reclutched/matches.db).
        #[arg(long)]
        db: Option<std::path::PathBuf>,
        /// Don't use colors (also off when the output is piped or NO_COLOR is set).
        #[arg(long)]
        no_color: bool,
        /// Show every stored number of the newest match instead of the list.
        #[arg(long)]
        detail: bool,
        /// Show every stored number of this match (see the # column).
        #[arg(long = "match", value_name = "ID")]
        match_id: Option<i64>,
    },
    /// Follow the game and save every finished match to the database.
    Track {
        /// Address of the Stats API websocket.
        #[arg(long, default_value = "127.0.0.1:49124")]
        addr: String,
        /// Database file (default: ~/.local/share/reclutched/matches.db).
        #[arg(long)]
        db: Option<std::path::PathBuf>,
        /// Settings file (default: ~/.config/reclutched/settings.json).
        #[arg(long)]
        settings: Option<std::path::PathBuf>,
    },
    /// Print raw Rocket League Stats API events to the terminal.
    Dump {
        /// Address of the Stats API websocket.
        #[arg(long, default_value = "127.0.0.1:49124")]
        addr: String,
        /// Print decoded events instead of the raw lines.
        #[arg(long)]
        parsed: bool,
    },
    /// Name playlist ids that aren't in the built-in table: play a match, then answer what it was.
    Map {
        /// Address of the Stats API websocket.
        #[arg(long, default_value = "127.0.0.1:49124")]
        addr: String,
        /// Where to save your names (default: ~/.config/reclutched/playlist_map.json).
        #[arg(long)]
        file: Option<std::path::PathBuf>,
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
        Command::Config { action, file } => {
            let path = file
                .or_else(Settings::default_path)
                .ok_or_else(|| anyhow::anyhow!("cannot find your home folder; pass --file"))?;
            run_config(action, &path)
        }
        Command::Dump { addr, parsed } => {
            // Run until Ctrl-C; `select!` returns as soon as either branch finishes.
            tokio::select! {
                _ = reclutched_daemon::dump::run(&addr, parsed) => Ok(()),
                _ = tokio::signal::ctrl_c() => Ok(()),
            }
        }
        Command::History {
            limit,
            db,
            no_color,
            detail,
            match_id,
        } => {
            let path = db
                .or_else(reclutched_core::rocketleague::storage::default_db_path)
                .ok_or_else(|| anyhow::anyhow!("cannot find your home folder; pass --db"))?;
            let store = reclutched_core::rocketleague::storage::Store::open(&path)?;
            let names = reclutched_core::rocketleague::playlist::default_map_path()
                .and_then(|p| reclutched_core::rocketleague::playlist::PlaylistMap::load(&p).ok())
                .unwrap_or_else(reclutched_core::rocketleague::playlist::PlaylistMap::with_builtin);
            use std::io::IsTerminal;
            let color = !no_color
                && std::io::stdout().is_terminal()
                && std::env::var_os("NO_COLOR").is_none();
            if detail || match_id.is_some() {
                reclutched_daemon::history::run_detail(&store, &names, match_id, color)
            } else {
                reclutched_daemon::history::run(&store, &names, limit, color)
            }
        }
        Command::Track { addr, db, settings } => {
            let paths = reclutched_daemon::track::Paths {
                db: db
                    .or_else(reclutched_core::rocketleague::storage::default_db_path)
                    .ok_or_else(|| anyhow::anyhow!("cannot find your home folder; pass --db"))?,
                settings: settings.or_else(Settings::default_path).ok_or_else(|| {
                    anyhow::anyhow!("cannot find your home folder; pass --settings")
                })?,
                playlist_map: reclutched_core::rocketleague::playlist::default_map_path(),
            };
            // `track` handles Ctrl-C itself so it can save the match in progress first.
            reclutched_daemon::track::run(&addr, paths).await
        }
        Command::Map { addr, file } => {
            let path = file
                .or_else(reclutched_core::rocketleague::playlist::default_map_path)
                .ok_or_else(|| anyhow::anyhow!("cannot find your home folder; pass --file"))?;
            tokio::select! {
                res = reclutched_daemon::map::run(&addr, path) => res,
                _ = tokio::signal::ctrl_c() => Ok(()),
            }
        }
    }
}

fn run_config(action: ConfigAction, path: &std::path::Path) -> anyhow::Result<()> {
    let mut settings = Settings::load(path)?;
    let (name, value) = match action {
        ConfigAction::List => {
            for (name, description) in TOGGLES {
                let state = if settings.get(name) == Some(true) {
                    "on "
                } else {
                    "off"
                };
                println!("{name:<16} {state}  {description}");
            }
            println!("\nsettings file: {}", path.display());
            return Ok(());
        }
        ConfigAction::Enable { name } => (name, true),
        ConfigAction::Disable { name } => (name, false),
    };
    if !settings.set(&name, value) {
        let known: Vec<&str> = TOGGLES.iter().map(|(n, _)| *n).collect();
        anyhow::bail!("unknown switch '{name}'. Known: {}", known.join(", "));
    }
    settings.save(path)?;
    println!("{name} is now {}", if value { "on" } else { "off" });
    Ok(())
}
