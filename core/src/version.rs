//! What this binary IS: a package version and the revision it was built from.
//!
//! # Why this exists
//!
//! v2 is deployed by cloning this repository at a hand-typed revision and
//! building on the box. Nothing surfaced drift, because no v2 binary could
//! state what it was: `tv-shell-core --version` was an unknown-argument error,
//! and both v2 crates are pinned `version = "0.0.0"`. "Is the thing running the
//! thing I built?" had no answer that did not involve trusting a shell-history
//! entry.
//!
//! # Why not `tv_shell_build_info`
//!
//! v1's metric (`daemon/src/metrics.rs`) shells out to `git` **at runtime**, so
//! on a box with a source tree it reports the tree's current HEAD rather than
//! the revision the running process was compiled from. Pull the tree forward
//! without restarting and it reports the new revision while serving the old
//! code. [`GIT_SHA`] comes from `build.rs` through `env!`, is fixed at compile
//! time, and therefore describes the binary rather than the directory it
//! happens to sit next to.
//!
//! # The version is `0.0.0` until a release stream exists
//!
//! [`VERSION`] is `CARGO_PKG_VERSION`, which this crate deliberately pins at
//! `0.0.0` (see `Cargo.toml`): no `core-v*` tag series exists yet, so there is
//! no released number to carry. That is exactly why [`GIT_SHA`] is the
//! load-bearing half today — and why the release workflow that stamps a tag
//! into the manifest needs no change here when it lands, since `VERSION`
//! already reads whatever the manifest says at build time.

/// The package version from `Cargo.toml`, stamped by a release build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The revision this binary was built from: a 12-character git sha, that sha
/// with `-dirty` appended, an injected `$TV_SHELL_BUILD_SHA`, or `unknown`.
///
/// Resolved by `build.rs`, which documents the precedence and the reason none
/// of its failure paths can fail the build.
pub const GIT_SHA: &str = env!("TV_SHELL_BUILD_SHA");

/// The binary this crate ships, named the way an operator types it.
pub const BINARY_NAME: &str = "tv-shell-core";

/// The one line `--version` prints: `tv-shell-core <version> (<sha>)`.
///
/// Identical in shape to `tv-shell-cec`'s, on purpose: an operator comparing
/// the two halves of a v2 deploy should be comparing shas, not parsing two
/// different formats.
pub fn version_string() -> String {
    format!("{BINARY_NAME} {VERSION} ({GIT_SHA})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_line_names_the_binary_the_version_and_the_revision() {
        let s = version_string();
        assert!(s.starts_with(BINARY_NAME), "{s}");
        assert!(s.contains(VERSION), "{s}");
        // The sha itself is NEVER asserted against a literal: it changes with
        // every commit, and a test pinned to one would fail on the commit that
        // introduced it. What must hold is that the rendered line carries
        // whatever the build stamped.
        assert!(s.contains(GIT_SHA), "{s}");
        assert!(s.ends_with(')'), "{s}");
    }

    #[test]
    fn the_stamp_is_present_and_says_something() {
        // An empty stamp is the failure mode that matters: `env!` would still
        // compile, `--version` would still exit 0, and the output would read
        // `tv-shell-core 0.0.0 ()` — an answer that looks like a version report
        // and carries no information. build.rs degrades to the literal
        // `unknown` instead, which is at least honest.
        assert!(!GIT_SHA.is_empty());
        assert!(!GIT_SHA.contains(char::is_whitespace), "{GIT_SHA}");
        assert!(!VERSION.is_empty());
    }
}
