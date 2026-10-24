//! Remembers which version of the source this program was built from and where that source
//! lives, so the app can tell whether a newer one exists and update itself.

use std::path::PathBuf;
use std::process::Command;

fn git(dir: &PathBuf, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let root = manifest
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap_or(manifest.clone());
    let commit = git(&root, &["rev-parse", "HEAD"]);
    let branch = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    println!(
        "cargo:rustc-env=RECLUTCHED_COMMIT={}",
        if commit.is_empty() {
            "unknown"
        } else {
            &commit
        }
    );
    println!(
        "cargo:rustc-env=RECLUTCHED_BRANCH={}",
        if branch.is_empty() {
            "unknown"
        } else {
            &branch
        }
    );
    println!("cargo:rustc-env=RECLUTCHED_SOURCE={}", root.display());
    // Build again when the checked-out commit changes.
    let git_dir = root.join(".git");
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    if let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) {
        if let Some(reference) = head.trim().strip_prefix("ref: ") {
            println!(
                "cargo:rerun-if-changed={}",
                git_dir.join(reference).display()
            );
        }
    }
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("packed-refs").display()
    );
}
