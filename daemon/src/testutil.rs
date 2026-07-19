//! Shared test-only scratch-path helper.
//!
//! The Rust quality Stop hook runs `cargo test --all-targets` inside a sandbox
//! that **denies writes to the system temp dir** (`std::env::temp_dir()`) —
//! any test that wrote there failed with `Os { code: 13, kind:
//! PermissionDenied }` while passing fine outside the hook.
//!
//! The fix (originally landed in `session_env::tests::scratch` by c70b0f4, now
//! extracted here so every module shares it) bases the scratch path on
//! [`std::env::current_exe`]'s parent instead of the system temp dir — the
//! running test binary's own `target/<profile>/deps/`, which is provably
//! writable because cargo just wrote the binary there. We deliberately do
//! **not** derive the base from `CARGO_MANIFEST_DIR/target`: in a cargo
//! **workspace** the real target dir is the workspace root's, not
//! `<crate>/target`, so `daemon/target/` may not exist and creating it fails
//! under the sandbox (which only permits writes to the pre-existing target
//! tree). Falls back to `CARGO_MANIFEST_DIR/target` if `current_exe()` is
//! unavailable.
//!
//! Every path is tagged by `tag` + pid + thread id, so parallel test threads
//! (and parallel test binaries) never collide.
//!
//! **Known residual limitation — this is NOT a complete cure.** Relocating off
//! the system temp dir is a strict improvement (that dir is *always* denied
//! under the hook's sandbox; `target/debug/deps/` usually is not), but it does
//! not make these tests deterministic under the sandbox. Confirmed
//! empirically 2026-07-19, in the *same* already-built worktree, across
//! repeated back-to-back `cargo test --all-targets` runs with no code changes
//! in between: most runs were fully green, but some failed one of
//! `session_env`'s tests, and others instead failed
//! `metrics::tests::write_atomic_creates_file_with_exact_contents` — with the
//! `EACCES` in the latter case coming from `metrics::write_atomic`'s own file
//! write, *after* this helper's `create_dir_all` for the same scratch
//! directory had already succeeded in the same test. That rules out "the
//! directory doesn't exist yet" as the sole explanation, and rules out fixing
//! it from test code alone: the denial can land on a production write inside
//! an already-successfully-created directory, and `metrics::write_atomic` is
//! not ours to add retry logic to. Treat any hook failure whose log shows
//! `PermissionDenied` on a path under `target/` as this same known flake — do
//! not chase it further locally; Linux CI (no such sandbox) is authoritative.
//!
//! **Not every `temp_dir()` call site is a candidate for this helper.**
//! `ipc::tests::end_to_end_commands_and_subscribe` binds a real Unix-domain
//! socket, whose path is capped at ~104-108 bytes by `sockaddr_un::sun_path`.
//! A path under a deeply nested worktree's `target/debug/deps/` routinely
//! blows that budget (a real nested-worktree checkout already measures ~90
//! bytes before the filename), so that one test stays on the real system temp
//! dir deliberately — see the comment at its call site.

use std::path::{Path, PathBuf};

/// The directory scratch paths are built under: the running test binary's own
/// `target/<profile>/deps/` (see module docs for why).
fn scratch_base() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"))
}

/// Build a unique scratch path for `tag`, suffixed with `suffix` (e.g.
/// `".json"`, or `""` for a directory/socket). Nothing is created on disk —
/// callers `create_dir_all`/`write`/etc. as needed and own their own cleanup,
/// matching how each call site behaved before this helper existed.
pub fn scratch_path(tag: &str, suffix: &str) -> PathBuf {
    scratch_base().join(format!(
        "{tag}-{}-{:?}{suffix}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// Create (`mkdir -p`) a fresh, empty scratch directory unique to this test
/// invocation, removing any stale leftover first. For tests that need a
/// directory to write multiple files/subdirs into.
pub fn scratch_dir(tag: &str) -> PathBuf {
    let dir = scratch_path(tag, "");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
