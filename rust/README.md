# game-shell-input (Rust)

A drop-in Rust replacement for `input/gamepad-input.py`. Same Unix socket, same
newline-delimited wire protocol (`docs/IPC_PROTOCOL.md`) — the QML shell is
unchanged. Phases 1–2 of [#28](https://github.com/jedwards1230/game-shell/issues/28).

## What it does

Grabs a gamepad via `EVIOCGRAB`, emits keyboard/mouse through uinput, and serves
the `grab`/`release`/`status`/`subscribe`/`get-bindings`/`set-binding`/
`capture-next`/`capture-cancel`/`kbd-log` protocol. Discovers an **arbitrary**
controller via the SDL GUID + bundled `SDL_GameControllerDB` (not just the
hardcoded Xbox pad), falling back to any `BTN_SOUTH` device.

Phase 2 added stateless commands that move parsing/serialization out of the QML
shell's inline `python3` one-liners and into the daemon:

- `list-apps` — scans XDG `.desktop` entries via the cross-platform
  `freedesktop-desktop-entry` crate, returns a compact JSON array.
- `get-config` / `set-config` — the daemon is the sole writer of
  `settings.json` (read-modify-write, compact JSON).
- `record-launch` / `get-recents` — maintains the recents file.

The QML side still opens the socket from a thin `python3` client but no longer
parses `.desktop` files or hand-formats config JSON.

## Layout

| File | Role |
|------|------|
| `protocol.rs` | Command parse + Event/response wire strings (bare text) |
| `config.rs` | Kernel codes, name tables, bindings, `settings.json` I/O |
| `apps.rs` | `.desktop` scan/parse → `list-apps` JSON (cross-platform) |
| `recents.rs` | Recents file I/O → `record-launch` / `get-recents` (cross-platform) |
| `device.rs` | SDL GUID + DB matching, device/keyboard discovery |
| `state.rs` | Control messages + pure input logic (velocity, deadzone, combos) |
| `input.rs` | Linux input runtime (evdev/uinput) — single state owner |
| `bluetooth.rs` | **Linux-only.** BlueZ actor via `bluer` — scan/pair/connect/trust + `bt:*` events |
| `network.rs` | **Linux-only.** NetworkManager **read** actor via `zbus` — connectivity / AP list / `net:*` events |
| `power.rs` | **Linux-only.** logind suspend + UPower battery via `zbus` — `power:*` events |
| `ipc.rs` | Unix-socket server, `broadcast` event fan-out, D-Bus command routing |
| `main.rs` | Runtime wiring + signals + D-Bus actor spawn |

`apps.rs` and `recents.rs` are pure Rust — fully unit-tested on macOS.

## Phase 3 — D-Bus backbone (`bluetooth.rs` / `network.rs` / `power.rs`)

Each subsystem is a long-lived async actor on the IPC runtime owning a single
`bluer::Session` or `zbus::Connection` (single-owner; no `Arc<Mutex>` across
`.await`). They answer request/response query commands and stream `bt:*` /
`net:*` / `power:*` events onto the existing `subscribe` bus. This deletes the
QML shell-outs that *read* system state:

- **Bluetooth** (`bluer`/BlueZ): `bt-power-*`, `bt-scan-*`, `bt-list`,
  `bt-connect`/`bt-disconnect`/`bt-pair`/`bt-trust`.
- **Wi-Fi reads** (`zbus`/NetworkManager): `net-status`, `net-wifi-list`,
  `net-wifi-rescan`. Wi-Fi **join** stays an `nmcli` shell-out.
- **Power/idle** (`zbus`/logind + UPower): `power-can-suspend`, `power-suspend`,
  `power-battery` (graceful "no battery" on a desktop).

These three modules are `#[cfg(target_os = "linux")]` (D-Bus is Linux-only) —
they are excluded from the macOS build, so the rest of the crate still compiles
and unit-tests there. The protocol parsing/response builders for every Phase 3
command stay cross-platform and are unit-tested. **The D-Bus paths can only be
exercised on-device** (no D-Bus on macOS / CI); each actor degrades to `error:*`
replies if BlueZ/NetworkManager/logind/UPower is absent, never panicking the
daemon. See `docs/IPC_PROTOCOL.md` for the full command/event reference.

## Build & test

The full binary only links on **Linux** (`evdev`/`uinput` are kernel
interfaces). `evdev` is a Linux-only dependency, so the portable modules still
compile and test on macOS:

```bash
cargo test            # runs everywhere (protocol/config/apps/recents/device/state/ipc)
cargo build --release # Linux only -> target/release/game-shell-input
```

## Deploy (later, on game-client-1)

1. `cargo build --release`
2. Install `target/release/game-shell-input` to `/opt/game-shell/bin/`
3. Switch the launch line in `scripts/game-shell-session.sh` (see the comment there)

Honors `GAME_SHELL_SOCK`, `GAMEPAD_VENDOR`/`GAMEPAD_PRODUCT` (exact-pin override),
and `GAME_SHELL_GAMECONTROLLERDB` (fuller controller DB).

## Status

The Python daemon stays the default until this is hardware-verified. Phase 3
(zbus/Bluetooth/Wi-Fi-read/power) is implemented above but **requires on-device
verification** — the D-Bus modules don't compile or run on macOS/CI. Phase 4
(Hyprland/CEC/health) is tracked in #28 and out of scope here.
