use clap::Parser;

/// Linux-native game stats tracker.
#[derive(Parser)]
#[command(version, about)]
struct Cli {}

fn main() {
    Cli::parse();
}
