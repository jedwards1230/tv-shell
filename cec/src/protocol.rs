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
//! * **Replies**: `ok` · `unknown` · `error:<msg>` · a bare compact JSON
//!   document as the whole line. No envelope, no `ok ` prefix on JSON.
//! * Anything interpolated into an error goes through [`sanitize_ipc`], because
//!   an embedded newline would split one reply into two and desync a
//!   line-reading client.
//!
//! The vocabulary is a **closed enum with an exhaustive match**, not a flat
//! string set — the §13 / 2026-09-07 decision. A new verb that nothing answers
//! is a compile error here, which is the property a `match` on `&str` does not
//! have.

/// Maximum accepted line length, matching v1 and the core.
pub const MAX_LINE: usize = 4096;

/// One parsed request.
///
/// **This step is read-only: no verb here transmits anything on the CEC bus.**
/// `wake`, `standby`, `input-claim`, `input-release`, `input-select`, the
/// `volume` family, `av-health`, `backend` and `backend-pin` are steps 4-7 of
/// the plan for jedwards1230/tv-shell#504 and are deliberately absent — an
/// unimplemented verb answers `unknown`, which is a client learning the truth
/// rather than a stub answering `ok`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Liveness. Replies `ok`.
    Ping,
    /// One [`crate::state::AvState`] snapshot as compact JSON.
    AvState,
    /// Not a verb this daemon has.
    Unknown,
}

impl Command {
    /// Parse one line. The trailing newline is already stripped by the codec;
    /// surrounding whitespace is trimmed, mirroring v1 and the core.
    pub fn parse(line: &str) -> Command {
        match line.trim() {
            "ping" => Command::Ping,
            "av-state" => Command::AvState,
            _ => Command::Unknown,
        }
    }
}

// NOTE: there is deliberately no `resp_usage` here, and no `*Usage` variant.
// Both of this step's verbs are bare reads, so there is no arity to get wrong
// and no usage arm to take — a usage builder would be a reply nothing can
// produce. `core/src/protocol.rs` carries the rule this follows: a type nothing
// constructs is dead code dressed as a contract, and the repo deletes dead code
// rather than parking it. The usage/unknown distinction comes back with the
// first verb that takes a body, `input-select <phys-addr>` in step 4.

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
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(Command::parse("  ping  "), Command::Ping);
        assert_eq!(Command::parse("\tav-state\n"), Command::AvState);
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
        // And the verbs that land in steps 4-7. `unknown` is the honest answer
        // until they do something.
        for later in [
            "wake",
            "standby",
            "input-claim",
            "input-release",
            "input-select 1.0.0.0",
            "volume up",
            "volume-state",
            "av-health",
            "backend",
            "backend-pin cec",
        ] {
            assert_eq!(Command::parse(later), Command::Unknown, "{later}");
        }
    }

    #[test]
    fn reply_grammar_matches_the_core() {
        assert_eq!(resp_ok(), "ok");
        assert_eq!(resp_unknown(), "unknown");
        assert_eq!(resp_error("boom"), "error:boom");
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
