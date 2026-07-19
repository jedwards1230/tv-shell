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
//! **A second, distinct failure mode found while chasing this**: it is not
//! only *whether* a new directory can be created under the sandbox — a
//! `create_dir_all` call can report `Ok` while leaving the directory with mode
//! `rw-------` (no search/execute bit), which then makes any later write
//! *inside* it fail with the same `EACCES`, even though creating the
//! directory itself "succeeded". Confirmed by directly inspecting a leftover
//! scratch directory after a hook-sandboxed run: `ls -ld` showed
//! `drw-------` plus a `com.apple.provenance` xattr, and `chmod 755` on it
//! immediately fixed access — so this is a real, inspectable mode bit, not a
//! transient deny. [`scratch_dir`] now explicitly re-`chmod`s to `0o755`
//! after creating its directory, and [`harden_dir`] is available for any test
//! that mkdirs a further nested directory of its own inside a scratch root
//! (see `session_env`'s `make_instance` and `daemon_config`'s
//! `with_temp_config_dir` for examples). Empirically this closed most of the
//! observed flake in an already-built worktree (0 failures across 15
//! back-to-back `cargo test --all-targets` runs here, vs. failing roughly
//! every other run before the `chmod`).
//!
//! **Known residual limitation — this is still NOT a guaranteed cure,
//! especially on a brand-new worktree's first run.** Every *new* directory
//! created during a sandboxed test run is independently exposed to the
//! mode-bit flake above; a shared helper can only harden the directories it
//! (or a caller that remembers to call [`harden_dir`]) actually creates. Two
//! test-owned nested `create_dir_all` calls were found and hardened this way
//! (`session_env`'s `make_instance` helper and its "present but empty hypr/
//! dir" case, and `daemon_config`'s `with_temp_config_dir`) — before that
//! extra hardening, 2 of 7 genuinely-fresh-worktree first runs failed at
//! exactly `make_instance`'s nested mkdir; after it, 5 of 5 follow-up
//! fresh-worktree first runs were green (small sample; a sandbox race, not a
//! proof of elimination). One site remains **structurally unfixable from
//! test code**: `config::atomic_write_creates_parent_and_replaces_atomically`
//! exercises `atomic_write`'s own `mkdir -p` of its parent — that's
//! **production** code we must not touch, and the whole point of the test is
//! that the directory does *not* exist before the call, so we can't pre-chmod
//! it either. Treat any hook failure whose log shows `PermissionDenied` on a
//! path under `target/` as this same known, environment-level sandbox flake —
//! do not chase it further locally; Linux CI (no such sandbox) is
//! authoritative.
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

/// Force `rwxr-xr-x` on `dir`. A no-op error is swallowed (non-unix, or the
/// directory vanished) — best-effort hardening only, see module docs.
pub fn harden_dir(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Create (`mkdir -p`) a fresh, empty scratch directory unique to this test
/// invocation, removing any stale leftover first. For tests that need a
/// directory to write multiple files/subdirs into.
pub fn scratch_dir(tag: &str) -> PathBuf {
    let dir = scratch_path(tag, "");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    harden_dir(&dir);
    dir
}
