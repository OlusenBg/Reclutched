//! Finding out that a newer version exists and installing it from the source folder this
//! program was built from (`git pull`, then `cargo install`), so you never type the commands.
//! The decisions are plain functions with tests; the git and cargo calls run on worker threads.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

pub const REPO_URL: &str = "https://github.com/OlusenBg/Reclutched.git";
/// The commit this program was built from ("unknown" outside a git checkout).
pub const COMMIT: &str = env!("RECLUTCHED_COMMIT");
pub const BRANCH: &str = env!("RECLUTCHED_BRANCH");
/// The source folder it was built in.
pub const SOURCE: &str = env!("RECLUTCHED_SOURCE");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The first seven characters of a commit.
pub fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

/// The branch updates follow: your choice, or the branch this build came from (`dev` or
/// otherwise `main`).
pub fn channel(setting: &str, build_branch: &str) -> &'static str {
    match setting {
        "main" => "main",
        "dev" => "dev",
        _ if build_branch == "dev" => "dev",
        _ => "main",
    }
}

/// The commit of `branch` in the output of `git ls-remote`.
pub fn parse_ls_remote(output: &str, branch: &str) -> Option<String> {
    let wanted = format!("refs/heads/{branch}");
    output.lines().find_map(|line| {
        let (sha, name) = line.split_once('\t')?;
        (name.trim() == wanted && sha.len() >= 7).then(|| sha.to_string())
    })
}

/// Whether two commits are the same (one may be a short form of the other).
pub fn same_commit(a: &str, b: &str) -> bool {
    a.len() >= 7 && b.len() >= 7 && (a.starts_with(b) || b.starts_with(a))
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    UpToDate,
    Available {
        commit: String,
    },
    /// We could not tell (offline, or a build that does not know its version).
    Unknown(String),
}

fn git_output(dir: Option<&Path>, args: &[&str], limit: Duration) -> Option<String> {
    let mut command = Command::new("git");
    command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(status) => {
                if !status.success() {
                    return None;
                }
                let mut text = String::new();
                std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
                return Some(text);
            }
            None if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// Is `source` a git checkout of this project?
pub fn source_is_usable(source: &Path) -> bool {
    source.join(".git").exists() && source.join("crates").join("reclutched-ui").is_dir()
}

/// Asks GitHub for the newest commit of the branch and decides whether we are behind it.
/// Blocks for a few seconds, so call it from a worker thread.
pub fn check(branch: &str) -> Check {
    let Some(listing) = git_output(
        None,
        &["ls-remote", REPO_URL, &format!("refs/heads/{branch}")],
        Duration::from_secs(20),
    ) else {
        return Check::Unknown("could not reach GitHub".to_string());
    };
    let Some(remote) = parse_ls_remote(&listing, branch) else {
        return Check::Unknown(format!("GitHub has no branch called {branch}"));
    };
    if COMMIT == "unknown" {
        return Check::Unknown("this build does not know which version it is".to_string());
    }
    if same_commit(COMMIT, &remote) {
        return Check::UpToDate;
    }
    // Different: are we behind, or ahead (unpublished work of your own)? Ask the source folder.
    let source = Path::new(SOURCE);
    if source_is_usable(source) {
        let fetched = git_output(
            Some(source),
            &["fetch", "--quiet", "origin", branch],
            Duration::from_secs(60),
        )
        .is_some();
        if fetched {
            if let Some(count) = git_output(
                Some(source),
                &["rev-list", "--count", &format!("{COMMIT}..{remote}")],
                Duration::from_secs(10),
            ) {
                return if count.trim() == "0" {
                    Check::UpToDate
                } else {
                    Check::Available { commit: remote }
                };
            }
        }
    }
    Check::Available { commit: remote }
}

/// One command of an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub program: String,
    pub args: Vec<String>,
    pub dir: PathBuf,
    /// What to tell the user while it runs.
    pub what: &'static str,
}

fn step(program: &str, args: &[&str], dir: &Path, what: &'static str) -> Step {
    Step {
        program: program.to_string(),
        args: args.iter().map(|a| (*a).to_string()).collect(),
        dir: dir.to_path_buf(),
        what,
    }
}

/// The commands that bring the source folder up to date and install the new version:
/// fast-forward only, so nothing of yours is ever overwritten.
pub fn plan(source: &Path, branch: &str, checked_out: &str, with_cli: bool) -> Vec<Step> {
    let mut steps = vec![step(
        "git",
        &["fetch", "origin", branch],
        source,
        "Fetching the new version",
    )];
    if checked_out != branch {
        steps.push(step(
            "git",
            &["checkout", branch],
            source,
            "Switching branch",
        ));
    }
    steps.push(step(
        "git",
        &["merge", "--ff-only", &format!("origin/{branch}")],
        source,
        "Updating the source",
    ));
    steps.push(step(
        "cargo",
        &["install", "--path", "crates/reclutched-ui", "--locked"],
        source,
        "Building Reclutched (a few minutes)",
    ));
    if with_cli {
        steps.push(step(
            "cargo",
            &["install", "--path", "crates/reclutched-cli", "--locked"],
            source,
            "Building the command line tool",
        ));
    }
    steps
}

/// A program installed by a package manager is updated by it, not by us.
pub fn looks_packaged(exe: &Path) -> bool {
    ["/usr/", "/opt/", "/app/", "/snap/"]
        .iter()
        .any(|prefix| exe.starts_with(prefix))
}

/// Whether we can update ourselves: the source folder must still be there and the program must
/// not come from a package.
pub fn can_install() -> Result<PathBuf, String> {
    let exe = clean_exe(std::env::current_exe().unwrap_or_default());
    if looks_packaged(&exe) {
        return Err("This copy was installed by a package manager: update it with that (for example your AUR helper).".to_string());
    }
    let source = PathBuf::from(SOURCE);
    if !source_is_usable(&source) {
        return Err(format!(
            "The source folder this was built from ({}) is gone. Clone the project again and run cargo install --path crates/reclutched-ui --locked.",
            source.display()
        ));
    }
    Ok(source)
}

/// What the kernel reports for a program that was replaced while it runs ends in " (deleted)".
pub fn clean_exe(exe: PathBuf) -> PathBuf {
    let text = exe.to_string_lossy();
    match text.strip_suffix(" (deleted)") {
        Some(clean) => PathBuf::from(clean),
        None => exe,
    }
}

/// The branch the source folder has checked out.
pub fn checked_out_branch(source: &Path) -> String {
    git_output(
        Some(source),
        &["rev-parse", "--abbrev-ref", "HEAD"],
        Duration::from_secs(5),
    )
    .map(|s| s.trim().to_string())
    .unwrap_or_default()
}

/// What an installation reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// What it is doing now ("Building Reclutched ...") and the latest line of output.
    Step(&'static str),
    Line(String),
    Done,
    Failed(String),
}

fn cargo_path() -> String {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut dirs = vec![home.join(".cargo").join("bin")];
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(dirs)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Runs the steps one after the other on a worker thread, reporting as it goes. The whole
/// output is written to `log`.
pub fn install(steps: Vec<Step>, log: PathBuf) -> Receiver<Progress> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if let Some(dir) = log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut log_file = std::fs::File::create(&log).ok();
        let mut tail: Vec<String> = Vec::new();
        for step in steps {
            let _ = tx.send(Progress::Step(step.what));
            // `sh -c 'exec "$0" "$@" 2>&1'` runs the program with its errors on the same pipe.
            let spawned = Command::new("sh")
                .arg("-c")
                .arg("exec \"$0\" \"$@\" 2>&1")
                .arg(&step.program)
                .args(&step.args)
                .current_dir(&step.dir)
                .env("PATH", cargo_path())
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("CARGO_TERM_COLOR", "never")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let Ok(mut child) = spawned else {
                let _ = tx.send(Progress::Failed(format!("could not run {}", step.program)));
                return;
            };
            if let Some(out) = child.stdout.take() {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    if let Some(file) = log_file.as_mut() {
                        let _ = std::io::Write::write_all(file, format!("{line}\n").as_bytes());
                    }
                    tail.push(line.clone());
                    if tail.len() > 8 {
                        tail.remove(0);
                    }
                    let _ = tx.send(Progress::Line(line));
                }
            }
            match child.wait() {
                Ok(status) if status.success() => {}
                _ => {
                    let _ = tx.send(Progress::Failed(tail.join("\n")));
                    return;
                }
            }
        }
        let _ = tx.send(Progress::Done);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channel_follows_the_choice_or_the_build() {
        assert_eq!(channel("auto", "dev"), "dev");
        assert_eq!(channel("auto", "main"), "main");
        assert_eq!(channel("auto", "claude/something"), "main");
        assert_eq!(channel("main", "dev"), "main");
        assert_eq!(channel("dev", "main"), "dev");
    }

    #[test]
    fn a_branch_is_found_in_git_ls_remote_output() {
        let out = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111\trefs/heads/dev\nbbbb2222bbbb2222bbbb2222bbbb2222bbbb2222\trefs/heads/main\n";
        assert_eq!(
            parse_ls_remote(out, "main").as_deref(),
            Some("bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222")
        );
        assert_eq!(parse_ls_remote(out, "nope"), None);
        // refs/heads/dev2 is not dev.
        assert_eq!(
            parse_ls_remote("cccc3333cccc\trefs/heads/dev2\n", "dev"),
            None
        );
        assert_eq!(parse_ls_remote("", "dev"), None);
    }

    #[test]
    fn commits_match_in_their_short_form_too() {
        assert!(same_commit("abcdef1234567890", "abcdef1"));
        assert!(same_commit("abcdef1", "abcdef1234567890"));
        assert!(!same_commit("abcdef1234", "abcdee1234"));
        assert!(!same_commit("unknown", "abcdef1234"));
        assert!(!same_commit("", ""));
        assert_eq!(short("abcdef1234567890"), "abcdef1");
        assert_eq!(short("abc"), "abc");
    }

    #[test]
    fn the_plan_only_fast_forwards() {
        let source = Path::new("/src/Reclutched");
        let on_branch = plan(source, "dev", "dev", false);
        let lines: Vec<String> = on_branch
            .iter()
            .map(|s| format!("{} {}", s.program, s.args.join(" ")))
            .collect();
        assert_eq!(
            lines,
            [
                "git fetch origin dev",
                "git merge --ff-only origin/dev",
                "cargo install --path crates/reclutched-ui --locked",
            ]
        );
        assert!(on_branch.iter().all(|s| s.dir == source));
        let other = plan(source, "main", "dev", true);
        assert_eq!(other[1].args, ["checkout", "main"]);
        assert_eq!(other.last().unwrap().args[2], "crates/reclutched-cli");
        // Nothing in the plan can throw work away.
        for s in other.iter().chain(on_branch.iter()) {
            assert!(!s
                .args
                .iter()
                .any(|a| a == "--force" || a == "reset" || a == "-f"));
        }
    }

    #[test]
    fn packaged_installs_are_left_to_the_package_manager() {
        assert!(looks_packaged(Path::new("/usr/bin/reclutched-ui")));
        assert!(!looks_packaged(Path::new(
            "/home/me/.cargo/bin/reclutched-ui"
        )));
        assert_eq!(
            clean_exe(PathBuf::from("/home/me/.cargo/bin/reclutched-ui (deleted)")),
            PathBuf::from("/home/me/.cargo/bin/reclutched-ui")
        );
        assert_eq!(
            clean_exe(PathBuf::from("/home/me/x")),
            PathBuf::from("/home/me/x")
        );
    }

    /// Needs the network: asks GitHub about the dev branch.
    #[test]
    #[ignore]
    fn github_can_be_asked() {
        let found = check("dev");
        assert!(
            matches!(found, Check::UpToDate | Check::Available { .. }),
            "{found:?}"
        );
    }

    #[test]
    fn the_installation_reports_each_step_and_stops_at_a_failure() {
        let dir = std::env::temp_dir();
        let ok = |what: &'static str, cmd: &str| Step {
            program: "sh".into(),
            args: vec!["-c".into(), cmd.into()],
            dir: dir.clone(),
            what,
        };
        let log = dir.join(format!("reclutched-update-test-{}.log", std::process::id()));
        let rx = install(
            vec![
                ok("one", "echo hello"),
                ok("two", "echo boom; exit 3"),
                ok("three", "echo never"),
            ],
            log.clone(),
        );
        let all: Vec<Progress> = rx.iter().collect();
        assert!(all.contains(&Progress::Step("one")));
        assert!(all.contains(&Progress::Line("hello".into())));
        assert!(all.contains(&Progress::Step("two")));
        assert!(!all.contains(&Progress::Step("three")));
        assert!(matches!(all.last(), Some(Progress::Failed(m)) if m.contains("boom")));
        assert!(std::fs::read_to_string(&log).unwrap().contains("hello"));
        let rx = install(vec![ok("fine", "true")], log.clone());
        assert_eq!(rx.iter().last(), Some(Progress::Done));
        let _ = std::fs::remove_file(log);
    }
}
