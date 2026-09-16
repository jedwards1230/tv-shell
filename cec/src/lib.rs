//! `tv-shell-cec` — the v2 AV-control daemon.
//!
//! A **new crate beside `core/`**, never an evolution of `daemon/` — V2_DESIGN
//! §13 Q12's precedent, and for the same reason the core needed it: v1's
//! `config.toml` root is `deny_unknown_fields`, so a v2 table added to it aborts
//! the v1 daemon at startup and the symptom reads as "v1 is broken". §11 makes
//! that a rule at every shared layer, so this daemon has its own config file
//! (`cec.toml`), its own socket (`tv-shell-v2-cec.sock`) and its own unit
//! (`tv-shell-v2-cec.service`), sharing none of them with v1 or the core.
//!
//! What this crate owns today:
//!
//! | Module | Owns |
//! |---|---|
//! | [`config`] | `~/.config/tv-shell/cec.toml` — a third file, separate from v1's and the core's |
//! | [`protocol`] | The IPC grammar, carried over from v1 and the core unchanged in contract (§4) |
//! | [`ipc`] | The Unix-socket server (0600), with the backend behind a trait so the tests need no adapter |
//! | [`backend`] | The seam: `linux-cec` types stop here and never reach [`ipc`] |
//! | [`state`] | The published snapshot, and the rule that `unknown` is never rendered as healthy or as `false` |
//! | [`ownership`] | **PURE**: the tri-state display-ownership model and the two transmit gates, ported from v1's `display_owner.rs` |
//! | [`action`] | **PURE**: a verb plus two observed addresses becomes a plan of messages, or a refusal that transmits nothing |
//! | [`volume`] | **PURE**: the volume/mute sequence — system-audio mode, an inseparable press/release pair, and success judged from the AVR's own report |
//! | [`kernel`] | `/dev/cecN`: open, configure, read the topology, and listen. **Linux-only** |
//! | [`health`] | **PURE**: the four observed facts, the tri-state derived from two of them, and the one rule the watchdog feed is gated on |
//! | [`failover`] | **PURE**: which backend is authoritative on the warm path, with hysteresis on both edges and a reason for every change |
//! | [`ip`] | The IP leg: a Wake-on-LAN magic packet and a Denon/Marantz telnet session, both behind a seam so no test dials anything |
//! | [`notify`] | `sd_notify` — `READY=1` for `Type=notify`, `WATCHDOG=1` for `WatchdogSec=` |
//!
//! # What this daemon puts on the bus, and when
//!
//! **Only on a client's request.** `wake`, `standby`, `input-claim`,
//! `input-release`, `input-select`, the `volume` family and `volume-state` are
//! the whole transmit surface; nothing fires on a timer, on a session event, or
//! on this daemon's own initiative. A shared bus may carry other playback
//! devices (a streaming box, a console) besides the television and the AVR, so
//! two rules are enforced by construction rather than by convention: a
//! `<Standby>` is always addressed and never broadcast
//! ([`action::StandbyTarget`] has no broadcast variant to pass), and `standby`
//! needs positive proof that this box holds the display before it transmits at
//! all ([`ownership::owns_display`]).
//!
//! The decision half is pure and lives beside the I/O: [`ownership`] holds the
//! two gates and the tri-state, [`action`] turns a verb plus two observed
//! addresses into either a plan or a refusal, and [`kernel::ops`] is the only
//! translation from this crate's vocabulary into `linux-cec` messages. CI has
//! no adapter and covers all three.
//!
//! # The IP leg is a complement first and a failover second
//!
//! [`ip`] carries two things CEC **cannot express at all** — a receiver's Zone 2
//! (`Z2OFF` has no CEC equivalent) and a cold wake of a television at mains
//! standby (`<Image View On>` reaches nothing there) — so its steps run *before*
//! the CEC steps of `wake` and `standby`, with a perfectly healthy bus. On top
//! of that, [`failover`] decides the **warm** path: which backend carries an
//! action when the adapter stops answering. `backend` publishes that decision
//! and `backend-pin` overrides it.
//!
//! The television's IP leg is **Wake-on-LAN only, write-only, with no state
//! read**. There is no LG webOS/SSAP client here and none should be written;
//! [`ip::wol`] carries the reasoning.
//!
//! It is a lib plus a thin bin for the same reason the daemon and the core are:
//! `pub` items in a library are public API and are never "dead", so
//! `clippy -D warnings` stays clean even where a module is not yet wired into
//! `main`.

pub mod action;
pub mod backend;
pub mod config;
pub mod failover;
pub mod health;
pub mod ip;
pub mod ipc;
pub mod notify;
pub mod ownership;
pub mod protocol;
pub mod state;
pub mod volume;

#[cfg(target_os = "linux")]
pub mod kernel;
