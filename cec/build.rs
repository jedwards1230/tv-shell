//! Stamp the git revision this binary was built from into the crate, at
//! COMPILE time.
//!
//! # Why a build script and not a runtime `git` call
//!
//! v1 answers "what is deployed?" with `tv_shell_build_info`, which shells out
//! to `git` from the running daemon (`daemon/src/metrics.rs`). On a box that
//! has a source tree — which is exactly how v2 is deployed today, by cloning at
//! a hand-typed revision and building in place — that reports the TREE's
//! current HEAD, not the revision the running binary was compiled from. Pull
//! the tree forward without restarting and it reports the new revision while
//! serving the old code: confidently wrong, which is worse than silent.
//!
//! A build script cannot drift that way. Whatever it resolves is baked in by
//! `env!`, so the answer travels with the artifact, survives being copied to a
//! box with no git and no source tree, and changes only when the binary does.
//!
//! # Precedence
//!
//! 1. `$TV_SHELL_BUILD_SHA`, if set and non-empty — the authoritative override.
//!    A release workflow (or any build from a tarball with no git history at
//!    all) injects the revision it knows, and nothing here second-guesses it.
//! 2. `git rev-parse --short=12 HEAD` in this crate's manifest dir, with
//!    `-dirty` appended when `git status --porcelain` reports anything. An
//!    uncommitted build is a real thing to know about on the couch.
//! 3. The literal `unknown`.
//!
//! **No timestamp, deliberately.** A build clock would make every rebuild
//! produce a different binary for no new information; the sha is the thing that
//! answers the question.
//!
//! # This script may never fail the build
//!
//! Every error path degrades to a less precise answer, never to a compile
//! error. A missing `git`, a shallow clone, a tarball, a sandbox that forbids
//! subprocesses: all of those are `unknown`, and `unknown` is a fine answer.
//! Refusing to build the AV daemon because a version string could not be
//! computed would be the tail wagging the dog.
//!
//! `core/build.rs` is the original; this is a deliberate copy. Sharing would mean a
//! third workspace crate existing only to be a `[build-dependencies]` entry for
//! two callers, which is more moving parts than the lines it would save. If a
//! third crate ever needs this, that trade flips.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // `cargo::` (double colon) is the current directive syntax; it has been
    // accepted since 1.77 and both crates declare `rust-version = "1.85"`, so
    // the old `cargo:` form is not needed here.
    println!("cargo::rerun-if-env-changed=TV_SHELL_BUILD_SHA");

    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    watch_git_head(&manifest_dir);

    println!(
        "cargo::rustc-env=TV_SHELL_BUILD_SHA={}",
        resolve(&manifest_dir)
    );
}

/// The stamped revision, by the precedence documented at the top of this file.
fn resolve(manifest_dir: &Path) -> String {
    if let Some(injected) = std::env::var("TV_SHELL_BUILD_SHA")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        return injected;
    }
    match git(manifest_dir, &["rev-parse", "--short=12", "HEAD"]) {
        Some(sha) if !sha.is_empty() => {
            // A failed dirty check must not turn a known sha into `unknown`:
            // the sha is the load-bearing half, and "we could not tell whether
            // it was dirty" is better served by saying nothing than by throwing
            // the revision away.
            match git(manifest_dir, &["status", "--porcelain"]) {
                Some(status) if !status.is_empty() => format!("{sha}-dirty"),
                _ => sha,
            }
        }
        _ => "unknown".to_string(),
    }
}

/// Run `git` in the crate, returning trimmed stdout on a clean exit.
///
/// `None` for every failure — no git on PATH, a non-zero exit, output that is
/// not UTF-8 — because each of them means the same thing to the caller.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}

/// Ask cargo to re-run this script when HEAD moves.
///
/// Without it a `git checkout` to another revision does NOT re-stamp: nothing
/// under `src/` changed, so cargo reuses the previous build and the binary
/// keeps claiming the revision it was first built at — the same
/// stale-provenance failure this whole file exists to remove, only baked in
/// instead of read live.
///
/// Every path is emitted only when it EXISTS. `rerun-if-changed` on a missing
/// path makes cargo consider the script dirty on every build, which would turn
/// a no-op `cargo build` into a rebuild of the crate.
fn watch_git_head(manifest_dir: &Path) {
    let Some(git_dir) = find_git_dir(manifest_dir) else {
        return;
    };
    let head = git_dir.join("HEAD");
    if !head.is_file() {
        return;
    }
    emit_rerun(&head);

    // A symbolic HEAD (`ref: refs/heads/main`) does not change when a commit
    // lands on that branch — the ref file does. Watch both. `packed-refs` is
    // the third spelling: a freshly cloned or gc'd repo has no loose ref file
    // at all, and the ref lives there instead.
    //
    // The refs live in the COMMON dir, which is the same directory in an
    // ordinary checkout and a different one inside a linked worktree.
    let Ok(head_text) = std::fs::read_to_string(&head) else {
        return;
    };
    let Some(reference) = head_text.trim().strip_prefix("ref:") else {
        // A detached HEAD holds the sha itself, so the file we already watch is
        // the whole story.
        return;
    };
    let common = common_dir(&git_dir);
    for candidate in [common.join(reference.trim()), common.join("packed-refs")] {
        if candidate.is_file() {
            emit_rerun(&candidate);
        }
    }
}

/// The git directory for this crate, or `None` outside a repository.
///
/// Handles the LINKED WORKTREE case, which is not exotic here — this repo's own
/// contributing guide puts branch work in a worktree directory. There `.git` is
/// a *file* containing `gitdir: <path>`, not a directory, so a check that
/// assumed a directory would silently watch nothing and stop re-stamping in
/// exactly the trees where branches are switched most.
fn find_git_dir(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }
        if dot_git.is_file() {
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let path = Path::new(text.trim().strip_prefix("gitdir:")?.trim()).to_path_buf();
            // The pointer may be relative to the directory holding the `.git`
            // file.
            return Some(if path.is_absolute() {
                path
            } else {
                dir.join(path)
            });
        }
    }
    None
}

/// Where a worktree's refs actually live: `$GIT_DIR/commondir` names it, and
/// its absence means this IS the common dir.
fn common_dir(git_dir: &Path) -> PathBuf {
    let Ok(text) = std::fs::read_to_string(git_dir.join("commondir")) else {
        return git_dir.to_path_buf();
    };
    let path = Path::new(text.trim());
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        git_dir.join(path)
    }
}

/// `rerun-if-changed` for one path, skipping a path that cannot be printed as
/// UTF-8 (cargo's directives are line-based and have no escaping).
fn emit_rerun(path: &Path) {
    if let Some(p) = path.to_str() {
        println!("cargo::rerun-if-changed={p}");
    }
}
