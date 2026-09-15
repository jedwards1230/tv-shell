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
//! | [`kernel`] | `/dev/cecN`: open, configure, read the topology, and listen. **Linux-only** |
//! | [`notify`] | `sd_notify` — `READY=1` for `Type=notify`, `WATCHDOG=1` for `WatchdogSec=` |
//!
//! # This step is READ-ONLY
//!
//! **This daemon performs no CEC transmits.** It opens the adapter, sets the
//! physical and logical addresses, reads the capabilities, and runs a follower
//! receive loop. That is the whole of it. The living-room bus carries an Apple
//! TV and a PS5 as well as the television and the AVR, so a stray transmit is a
//! real-world side effect on someone's evening — which is why the ordering in
//! [`kernel::device`] is pinned by a comment rather than left to chance.
//!
//! Explicitly **not** here yet, each a later step of the plan for
//! jedwards1230/tv-shell#504: power and input switching (`wake`, `standby`,
//! `input-claim`/`-release`/`-select`) and the pure ownership gates that must
//! precede them; volume and system-audio mode; the health state machine and
//! `av-health`; and the IP recovery leg (Denon/Marantz telnet, WoL) with the
//! failover decision. Each lands with the module that reads it.
//!
//! It is a lib plus a thin bin for the same reason the daemon and the core are:
//! `pub` items in a library are public API and are never "dead", so
//! `clippy -D warnings` stays clean even where a module is not yet wired into
//! `main`.

pub mod backend;
pub mod config;
pub mod ipc;
pub mod notify;
pub mod protocol;
pub mod state;

#[cfg(target_os = "linux")]
pub mod kernel;
