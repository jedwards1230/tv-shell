//! What this binary IS: a package version and the revision it was built from.
//!
//! The reasoning is the core's, verbatim, and deliberately so — see
//! `core/src/version.rs`. In short:
//!
//! * v2 is deployed by cloning this repository at a hand-typed revision and
//!   building on the box, and no v2 binary could state what it was.
//! * [`GIT_SHA`] is stamped by `build.rs` at COMPILE time, so unlike v1's
//!   `tv_shell_build_info` — which shells out to `git` at runtime and therefore
//!   reports the source tree's HEAD rather than the running code's — it cannot
//!   drift away from the artifact it describes.
//! * [`VERSION`] is `0.0.0` until a `cec-v*` (or shared v2) tag series exists;
//!   the sha is the load-bearing half in the meantime.
//!
//! It matters a little more here than in the core. This daemon is the one with
//! restart authority delegated to `WatchdogSec=`, and "which build is on the
//! bus?" is the first question of every CEC incident.

/// The package version from `Cargo.toml`, stamped by a release build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The revision this binary was built from: a 12-character git sha, that sha
/// with `-dirty` appended, an injected `$TV_SHELL_BUILD_SHA`, or `unknown`.
pub const GIT_SHA: &str = env!("TV_SHELL_BUILD_SHA");

/// The binary this crate ships, named the way an operator types it.
pub const BINARY_NAME: &str = "tv-shell-cec";

/// The one line `--version` prints: `tv-shell-cec <version> (<sha>)`.
///
/// Identical in shape to `tv-shell-core`'s, on purpose: an operator comparing
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
        // Never asserted against a literal sha: it changes with every commit.
        assert!(s.contains(GIT_SHA), "{s}");
        assert!(s.ends_with(')'), "{s}");
    }

    #[test]
    fn the_stamp_is_present_and_says_something() {
        // An empty stamp still compiles and still exits 0, and prints
        // `tv-shell-cec 0.0.0 ()` — a version report carrying no information.
        // build.rs degrades to the literal `unknown` instead.
        assert!(!GIT_SHA.is_empty());
        assert!(!GIT_SHA.contains(char::is_whitespace), "{GIT_SHA}");
        assert!(!VERSION.is_empty());
    }
}
