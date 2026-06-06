#!/usr/bin/env bash
# Canonical release build for the game-shell input daemon.
#
# Single source of truth for the daemon's on-device / deploy build flags (Cargo
# features, profile). Consumers that build the daemon on a device — the dev HTTP
# bridge's `POST /dev/build` (daemon/src/http.rs) and the homelab-ansible
# `game_client_common` role — invoke THIS script instead of hardcoding cargo
# flags, so the feature set is decided here in the repo, not in each consumer.
#
# The `cec` feature is intentionally NOT a default Cargo feature: a bare
# `cargo build` / `cargo test` (CI default leg, contributor and macOS dev boxes)
# stays free of the libcec C toolchain. This deploy build is where we opt in.
#
# Env overrides:
#   GAME_SHELL_ROOT      repo root (default: parent of this script's directory)
#   GAME_SHELL_FEATURES  cargo --features list (default: "cec")
set -euo pipefail

ROOT="${GAME_SHELL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
FEATURES="${GAME_SHELL_FEATURES:-cec}"

cd "$ROOT/daemon"
echo "build-daemon: cargo build --release --features ${FEATURES} (in ${ROOT}/daemon)"
exec cargo build --release --features "${FEATURES}"
