//! Stamps `PTY_VERSION` = `<CARGO_PKG_VERSION>+<short-sha>` into the binary,
//! e.g. `0.13.0-rust+1a2b3c4`.
//!
//! The short SHA comes from `PTY_BUILD_SHA` when set (nix builds have no
//! `.git`), else `git rev-parse --short HEAD` run in this crate's repository,
//! else `unknown`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=PTY_BUILD_SHA");

    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let sha = match std::env::var("PTY_BUILD_SHA") {
        Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => git_short_sha(&manifest_dir).unwrap_or_else(|| "unknown".to_string()),
    };

    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    println!("cargo:rustc-env=PTY_VERSION={version}+{sha}");
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn git_short_sha(repo: &Path) -> Option<String> {
    let sha = git(repo, &["rev-parse", "--short", "HEAD"])?;

    // Rebuild when HEAD moves: watch HEAD itself and, for a symbolic HEAD, the
    // branch ref it points at. `--git-path` resolves both for worktrees.
    // Only existing files are declared; a missing path would make cargo
    // rerun the script on every build.
    if let Some(head) = git(repo, &["rev-parse", "--git-path", "HEAD"]) {
        let head = repo.join(head);
        if head.exists() {
            println!("cargo:rerun-if-changed={}", head.display());
        }
        if let Ok(content) = std::fs::read_to_string(&head)
            && let Some(target) = content.trim().strip_prefix("ref: ")
            && let Some(ref_path) = git(repo, &["rev-parse", "--git-path", target])
        {
            let ref_path = repo.join(ref_path);
            if ref_path.exists() {
                println!("cargo:rerun-if-changed={}", ref_path.display());
            }
        }
    }
    Some(sha)
}
