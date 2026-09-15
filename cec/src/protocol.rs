//! Wire protocol — command parsing and reply builders.
//!
//! **The core's contract, carried over unchanged.** V2_DESIGN §4 lists "the
//! Unix-socket IPC framing and reply grammar" among the things carried over from
//! v1 unchanged in contract, and `core/src/protocol.rs` is the v2 statement of
//! it. A third daemon speaking a fourth grammar would make the §11 "beside, not
//! instead" separation cost a client one parser per socket:
//!
//! * **Framing**: newline-delimited text, one command per line, one reply line
//!   per command, 4096-byte maximum line.
//! * **Tokenization**: whitespace-split, no quoting. A verb with a body must be
//!   followed by whitespace, so `av-stateX` is not `av-state`.
//! * **Replies**: `ok` · `unknown` · `error:<msg>` · `refused:<why>` · a bare
//!   compact JSON document as the whole line. No envelope, no `ok ` prefix on
//!   JSON.
//! * Anything interpolated into an error goes through [`sanitize_ipc`], because
//!   an embedded newline would split one reply into two and desync a
//!   line-reading client.
//!
//! The vocabulary is a **closed enum with an exhaustive match**, not a flat
//! string set — the §13 / 2026-09-07 decision. A new verb that nothing answers
//! is a compile error here, which is the property a `match` on `&str` does not
//! have.
//!
//! # `refused:` — the one addition to the reply grammar
//!
//! v1 replied `ok` when its ownership gate skipped a transmit, on the reasoning
//! that a skip performs zero transmits and therefore fails at nothing. That is
//! true and it is still the wrong reply: it makes "we deliberately declined to
//! power off your television" read exactly like "we powered off your
//! television", and no later observation separates them — on a shared bus the
//! set can go off for somebody else's reason.
//!
//! `error:` is not right either. It says a fault occurred, which sends an
//! operator after a wedged adapter; that is the `cec-health` failure shape this
//! whole design exists to remove, one layer down.
//!
//! So a refusal gets its own token: **`refused:<why>` means the daemon
//! deliberately did not act, nothing is broken, and ZERO messages reached the
//! bus.** The zero-transmit half is not a convention — it is enforced by
//! construction in [`crate::action`], where every gate runs before any message
//! is built.

use crate::state::PhysAddr;
use crate::volume::VolumeAction;

/// Maximum accepted line length, matching v1 and the core.
pub const MAX_LINE: usize = 4096;

/// Usage line for `input-select`.
pub const INPUT_SELECT_USAGE: &str = "input-select <phys-addr>  (e.g. input-select 1.0.0.0)";

/// Usage line for `volume`.
pub const VOLUME_USAGE: &str = "volume up|down|mute|unmute";

/// One parsed request.
///
/// `av-health`, `backend` and `backend-pin` are steps 6-7 of the plan for
/// jedwards1230/tv-shell#504 and are deliberately absent — an unimplemented verb
/// answers `unknown`, which is a client learning the truth rather than a stub
/// answering `ok`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Liveness. Replies `ok`.
    Ping,
    /// One [`crate::state::AvState`] snapshot as compact JSON.
    AvState,
    /// Power the chain on and become the active source.
    Wake,
    /// Put the television and the AVR into standby. Gated on positive proof of
    /// ownership, so it can reply `refused:`.
    Standby,
    /// Become the active source without touching power.
    InputClaim,
    /// Give the display up.
    InputRelease,
    /// Hand the display to the named physical address.
    InputSelect(PhysAddr),
    /// `input-select` with a missing, malformed or over-long body.
    ///
    /// **Distinct from [`Command::Unknown`], and distinct from a silent
    /// default.** `input-select` with no argument is a client that knows the
    /// verb and got the call wrong; answering `unknown` would send them looking
    /// for a verb that exists. And a malformed address must never fall back to
    /// "ours" or to any other default — a `<Set Stream Path>` to a port that
    /// does not exist fails *silently* on the bus.
    InputSelectUsage,
    /// Step the AVR's volume, or set its mute flag.
    Volume(VolumeAction),
    /// The AVR's level and mute flag, with the source of each named.
    VolumeState,
    /// `volume` with a missing or unknown argument.
    ///
    /// **Distinct from [`Command::Unknown`], and never a silent default.** A
    /// bare `volume`, or a `volume louder`, is a client that knows the verb and
    /// got the call wrong. Defaulting it to `up` — or to anything — would make a
    /// typo reply `ok` for a change the client never asked for, which is exactly
    /// the confusion this design removes.
    VolumeUsage,
    /// Not a verb this daemon has.
    Unknown,
}

impl Command {
    /// Parse one line. The trailing newline is already stripped by the codec;
    /// surrounding whitespace is trimmed, mirroring v1 and the core.
    ///
    /// Tokenization is whitespace-split with no quoting, so a verb is a whole
    /// word: `av-stateX` is a different word and `standby now` is the wrong
    /// arity for a bare verb. Both answer `unknown` rather than being coerced
    /// into the nearest match.
    pub fn parse(line: &str) -> Command {
        let mut parts = line.split_whitespace();
        let Some(verb) = parts.next() else {
            return Command::Unknown;
        };
        let body: Vec<&str> = parts.collect();
        match (verb, body.as_slice()) {
            ("ping", []) => Command::Ping,
            ("av-state", []) => Command::AvState,
            ("wake", []) => Command::Wake,
            ("standby", []) => Command::Standby,
            ("input-claim", []) => Command::InputClaim,
            ("input-release", []) => Command::InputRelease,
            ("input-select", [addr]) => addr
                .parse::<PhysAddr>()
                .map_or(Command::InputSelectUsage, Command::InputSelect),
            ("input-select", _) => Command::InputSelectUsage,
            ("volume", [word]) => {
                VolumeAction::parse(word).map_or(Command::VolumeUsage, Command::Volume)
            }
            // Covers both a bare `volume` and `volume up down`: neither is a
            // request this daemon can act on, and neither may be coerced into
            // one.
            ("volume", _) => Command::VolumeUsage,
            ("volume-state", []) => Command::VolumeState,
            _ => Command::Unknown,
        }
    }
}

// ---------------------------------------------------------------------------
// Response builders (the exact reply strings, sans trailing newline).
// ---------------------------------------------------------------------------

/// Success with no payload.
pub fn resp_ok() -> String {
    "ok".to_string()
}

/// The client sent a verb this daemon does not have.
pub fn resp_unknown() -> String {
    "unknown".to_string()
}

/// Every error reply. `msg` is free text, sanitized to one line.
pub fn resp_error(msg: &str) -> String {
    format!("error:{}", sanitize_ipc(msg))
}

/// Wrong arity or a malformed body. An `error:`, because the client got the call
/// wrong — matching `core/src/protocol.rs`'s builder exactly.
pub fn resp_usage(usage: &str) -> String {
    resp_error(&format!("usage: {usage}"))
}

/// The daemon deliberately did not act. **Zero messages reached the bus.**
///
/// Its own token rather than `ok` or `error:` — see the module docs. `why` is
/// free text and is sanitized to one line like every other interpolated reply.
pub fn resp_refused(why: &str) -> String {
    format!("refused:{}", sanitize_ipc(why))
}

/// Serialize a payload as the whole reply line, degrading to an error reply.
pub fn resp_json<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_string(value) {
        Ok(json) => sanitize_ipc(&json),
        Err(e) => resp_error(&format!("serialize failed: {e}")),
    }
}

/// Strip control characters from anything destined for a reply line.
///
/// The wire protocol is newline-delimited and clients read it line-by-line, so a
/// `\n` or `\r` embedded in an error body would split one reply into several and
/// desync the client. Keeping every reply on one line preserves framing. Pure —
/// unit-tested.
pub fn sanitize_ipc(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_verbs_parse() {
        assert_eq!(Command::parse("ping"), Command::Ping);
        assert_eq!(Command::parse("av-state"), Command::AvState);
        assert_eq!(Command::parse("wake"), Command::Wake);
        assert_eq!(Command::parse("standby"), Command::Standby);
        assert_eq!(Command::parse("input-claim"), Command::InputClaim);
        assert_eq!(Command::parse("input-release"), Command::InputRelease);
    }

    #[test]
    fn input_select_parses_its_physical_address() {
        assert_eq!(
            Command::parse("input-select 1.0.0.0"),
            Command::InputSelect("1.0.0.0".parse().unwrap())
        );
        assert_eq!(
            Command::parse("  input-select   2.5.0.0  "),
            Command::InputSelect("2.5.0.0".parse().unwrap())
        );
    }

    /// **The rule: a malformed physical address is a usage error, never a
    /// silent default.**
    ///
    /// A `<Set Stream Path>` naming a port that does not exist fails silently on
    /// the bus — nothing NAKs it and nothing changes — so a body this daemon
    /// cannot parse must be refused at the parser, where the client still learns
    /// about it.
    ///
    /// Mutation-check (run 2026-09-14): make the parse fall back to a default
    /// address (`unwrap_or(PhysAddr::INVALID)` or the configured one) and every
    /// row here fails.
    #[test]
    fn a_malformed_input_select_body_is_a_usage_error() {
        for line in [
            "input-select",
            "input-select ",
            "input-select x",
            "input-select 1.0.0",
            "input-select 1.0.0.0.0",
            "input-select 10.0.0.0",
            "input-select 0x1000",
            "input-select 1.0.0.g",
            "input-select 1.0.0.0 extra",
            "input-select 1.0.0.0 2.0.0.0",
        ] {
            assert_eq!(
                Command::parse(line),
                Command::InputSelectUsage,
                "{line:?} must be a usage error"
            );
        }
    }

    #[test]
    fn the_volume_family_parses() {
        assert_eq!(
            Command::parse("volume up"),
            Command::Volume(VolumeAction::Up)
        );
        assert_eq!(
            Command::parse("volume down"),
            Command::Volume(VolumeAction::Down)
        );
        assert_eq!(
            Command::parse("volume mute"),
            Command::Volume(VolumeAction::Mute)
        );
        assert_eq!(
            Command::parse("  volume   unmute  "),
            Command::Volume(VolumeAction::Unmute)
        );
        assert_eq!(Command::parse("volume-state"), Command::VolumeState);
    }

    /// **The rule: `volume` with a missing or unknown argument is a USAGE error
    /// — never a silent default, and never `unknown`.**
    ///
    /// A typo that read as `ok` would tell a caller the volume had changed when
    /// nothing had; answering `unknown` would send a client looking for a verb
    /// that exists.
    ///
    /// Mutation-check (run 2026-09-14): make the bare-`volume` arm fall back to
    /// `Command::Volume(VolumeAction::Up)` and every row here fails, along with
    /// the `ipc` twin.
    #[test]
    fn a_missing_or_unknown_volume_argument_is_a_usage_error() {
        for line in [
            "volume",
            "volume ",
            "  volume  ",
            "volume louder",
            "volume UP",
            "volume 1",
            "volume +",
            "volume up down",
            "volume up 1",
            "volume mute unmute",
        ] {
            assert_eq!(
                Command::parse(line),
                Command::VolumeUsage,
                "{line:?} must be a usage error"
            );
        }
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(Command::parse("  ping  "), Command::Ping);
        assert_eq!(Command::parse("\tav-state\n"), Command::AvState);
        assert_eq!(Command::parse("  standby "), Command::Standby);
    }

    /// **The rule: a verb is a whole word.**
    ///
    /// `av-stateX` is a different word, and answering it with a topology
    /// snapshot would mean a client's typo silently succeeds — the shape the
    /// core's own `word_boundaries_are_enforced` pins. Both verbs here are bare
    /// reads, so the boundary also means nothing may follow them.
    ///
    /// Mutation-check (run 2026-09-14): replace the `match` arms with
    /// `starts_with` prefix tests and every assertion below fails.
    #[test]
    fn word_boundaries_are_enforced() {
        for line in [
            "av-stateX",
            "av-state-full",
            "av-state 1",
            "av-states",
            "pingpong",
            "ping 1",
            "avstate",
            "wakeX",
            "wake up",
            "standbyX",
            "standby now",
            "input-claimX",
            "input-claim 1.0.0.0",
            "input-releaseX",
            "input-release 1.0.0.0",
            "input-selectX 1.0.0.0",
            "input-selects 1.0.0.0",
            "volumeX up",
            "volumes up",
            "volume-stateX",
            "volume-states",
            "volume-state 1",
            "volumestate",
        ] {
            assert_eq!(
                Command::parse(line),
                Command::Unknown,
                "{line:?} is not one of this daemon's verbs"
            );
        }
    }

    #[test]
    fn unknown_verbs_are_unknown() {
        assert_eq!(Command::parse(""), Command::Unknown);
        assert_eq!(Command::parse("frobnicate"), Command::Unknown);
        // v1 CEC verbs this daemon deliberately does not answer: `cec-health`'s
        // inference chain is what V2_DESIGN §9 retires, and `cec-scan` is a bus
        // write wearing a read's name.
        assert_eq!(Command::parse("cec-health"), Command::Unknown);
        assert_eq!(Command::parse("cec-scan"), Command::Unknown);
        // And the verbs that land in steps 6-7. `unknown` is the honest answer
        // until they do something.
        for later in ["av-health", "backend", "backend-pin cec"] {
            assert_eq!(Command::parse(later), Command::Unknown, "{later}");
        }
    }

    #[test]
    fn reply_grammar_matches_the_core() {
        assert_eq!(resp_ok(), "ok");
        assert_eq!(resp_unknown(), "unknown");
        assert_eq!(resp_error("boom"), "error:boom");
        assert_eq!(resp_usage("a <b>"), "error:usage: a <b>");
    }

    /// **The rule: a refusal is not a success and not an error.**
    ///
    /// Three distinct tokens, so a caller can tell "we did it", "we deliberately
    /// did not" and "something is broken" apart. A refusal that read identically
    /// to a success is exactly the confusion this design removes.
    #[test]
    fn a_refusal_is_its_own_token() {
        let refused = resp_refused("someone else holds the display");
        assert_eq!(refused, "refused:someone else holds the display");
        assert_ne!(refused, resp_ok());
        assert!(!refused.starts_with("error:"));
        assert!(!refused.starts_with("ok"));
        // Sanitized like every other interpolated reply — a newline in a
        // refusal reason would split one reply into two.
        assert_eq!(resp_refused("a\nb"), "refused:a b");
        assert!(!resp_refused("x\r\ny").contains('\n'));
    }

    #[test]
    fn json_replies_are_the_whole_line_with_no_envelope() {
        let reply = resp_json(&serde_json::json!({"a": 1}));
        assert_eq!(reply, r#"{"a":1}"#);
        assert!(!reply.starts_with("ok"));
    }

    #[test]
    fn control_characters_never_reach_the_wire() {
        assert_eq!(sanitize_ipc("a\nb"), "a b");
        assert_eq!(sanitize_ipc("a\r\nb\tc"), "a  b c");
        assert_eq!(sanitize_ipc("plain"), "plain");
        // The framing invariant: no reply may contain a newline.
        for s in ["a\nb", "\r", "x\u{0}y"] {
            assert!(!resp_error(s).contains('\n'));
            assert!(!resp_error(s).contains('\r'));
        }
    }

    #[test]
    fn every_error_reply_is_prefixed_and_single_line() {
        let replies = [
            resp_error("x"),
            // A map keyed by a tuple is a serde_json hard error ("key must be a
            // string"), so this exercises the degrade-to-error path in
            // `resp_json` rather than a happy value.
            resp_json(&std::collections::BTreeMap::from([((1u8, 2u8), 3u8)])),
        ];
        for r in replies {
            assert!(r.starts_with("error:"), "{r}");
            assert_eq!(r.lines().count(), 1, "{r}");
        }
    }

    #[test]
    fn max_line_matches_v1_and_the_core() {
        assert_eq!(MAX_LINE, 4096);
    }
}
