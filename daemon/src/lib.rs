//! game-shell-input library: every daemon module lives here as public API.
//!
//! The binary (`main.rs`) only wires these together. Exposing the modules as a
//! library's public surface keeps the cross-platform modules out of dead-code
//! analysis on non-Linux hosts — where the Linux-only `main` (and the evdev/
//! D-Bus/Hyprland modules) are `cfg`-excluded, so a bin-only crate would flag
//! the entire daemon as unused. As a lib, `pub` items are the public API and
//! are never "dead", so `cargo clippy -D warnings` is clean on macOS too.

pub mod apps;
pub mod config;
pub mod controllerdb;
pub mod device;
pub mod health;
pub mod ipc;
pub mod protocol;
pub mod recents;
pub mod state;
pub mod system;

// Session-environment self-discovery and daemon.env loading (#165).
// Cross-platform: no Linux-only imports.
pub mod session_env;

// LAN HTTP control bridge — cross-platform (tokio only; no Linux-only imports).
pub mod http;

// evdev/uinput input runtime — Linux kernel interfaces.
#[cfg(target_os = "linux")]
pub mod input;

// D-Bus backbone (Linux-only): BlueZ via `bluer`; NetworkManager/logind/UPower
// via `zbus`.
#[cfg(target_os = "linux")]
pub mod bluetooth;
#[cfg(target_os = "linux")]
pub mod network;
#[cfg(target_os = "linux")]
pub mod power;

// Hyprland IPC over its Unix sockets directly (no crate; Linux-only).
#[cfg(target_os = "linux")]
pub mod hyprland;

// HDMI-CEC actor (#94): persistent libcec connection via cec-rs. Linux-only AND
// feature-gated (`cec`) — libcec-sys links a C lib, so default builds exclude it
// to keep the no-system-C-deps invariant (evdev/zbus/bluer are pure Rust).
#[cfg(all(target_os = "linux", feature = "cec"))]
pub mod cec;

// File-watch actor: inotify-watches settings.json for external edits and
// broadcasts config:changed. Uses notify-debouncer-full (Linux-only crate).
#[cfg(target_os = "linux")]
pub mod watch;

// logind session-active watcher: releases the gamepad grab while our session is
// backgrounded (e.g. VT-switched away) so the foreground DE gets the controller.
// zbus + logind; Linux-only.
#[cfg(target_os = "linux")]
pub mod session;
