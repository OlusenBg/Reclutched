//! Prints every Stats API message to the terminal.

use reclutched_core::rocketleague::parse_message;

use crate::source;

/// With `parsed`, prints the decoded event instead of the raw line.
/// Runs until the process is stopped.
pub async fn run(addr: &str, parsed: bool) {
    let mut rx = source::spawn(addr);
    while let Some(raw) = rx.recv().await {
        if !parsed {
            println!("{raw}");
            continue;
        }
        match parse_message(&raw) {
            Ok(event) => println!("{event:?}"),
            // Show the problem and the line, so a new payload shape is easy to report.
            Err(err) => eprintln!("could not parse ({err}): {raw}"),
        }
    }
}
