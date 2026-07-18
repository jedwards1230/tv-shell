#!/usr/bin/env bash
# Canonical release build for the tv-shell web control panel.
#
# Single source of truth for the panel's on-device / deploy build flags. Mirrors
# scripts/build-daemon.sh's role for tv-shell-input, but the panel is pure Rust
# (axum + askama + reqwest-rustls) — no cec/mcp Cargo features and no system C
# libs to opt into, so there is no --features machinery here: just a plain
# release build of the `panel` workspace member.
#
# Env overrides (legacy GAME_SHELL_* names honored as a fallback):
#   TV_SHELL_ROOT      repo root (default: parent of this script's directory)
set -euo pipefail

ROOT="${TV_SHELL_ROOT:-${GAME_SHELL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}}"

# The repo is a Cargo workspace (daemon/ + host/ + protocol/ + panel/). Build the
# panel package explicitly (`-p tv-shell-panel`) from the workspace root so no
# other member is dragged into the build, and so the output lands at the
# workspace-root `target/release/` (one shared target dir).
cd "$ROOT"
echo "build-panel: cargo build --release -p tv-shell-panel (in ${ROOT})"
exec cargo build --release -p tv-shell-panel
