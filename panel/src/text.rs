//! Small text-processing helpers shared by more than one log pane. Kept in
//! its own module rather than duplicated per-page (unlike the intentional
//! page-local `esc`/`pretty_json_text` copies elsewhere) since both the
//! shell log and the daemon log go through the exact same ANSI-stripping
//! step in `pages::logs`.

/// Strip ANSI escape sequences from `s` — SGR color/style codes
/// (`\x1b[32m`), cursor movement/erase (`\x1b[2J`, `\x1b[1A`, ...), and other
/// CSI sequences, plus simpler two-byte escapes. Log output piped through a
/// non-tty (journalctl, the daemon's dev-log bridge) still carries the
/// terminal color codes the original process emitted; this renders as
/// garbled `^[[32m`-style noise in a plain `<pre>` block, so both log panes
/// in `pages::logs` strip it before rendering.
///
/// Also strips "bare" CSI sequences that have already lost their leading
/// `ESC` byte somewhere in transit — some log transports (observed on the
/// daemon's dev-log bridge) drop the raw `\x1b` control byte but leave the
/// rest of the sequence behind, e.g. `[33m WARN[97m qt.svg.draw[0m: ...`.
/// See [`strip_bare_ansi`] for the (deliberately narrow) match rule.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            // CSI: ESC '[' <parameter bytes> <intermediate bytes> <final byte
            // 0x40-0x7E> — covers SGR color codes and cursor movement/erase.
            Some('[') => {
                chars.next();
                for c2 in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c2) {
                        break;
                    }
                }
            }
            // Any other two-byte escape (e.g. ESC 'M' reverse-index) —
            // consume the one following byte.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    strip_bare_ansi(&out)
}

/// Strip CSI sequences that are missing their leading `ESC` byte —
/// `[<params>m` (SGR color/style, e.g. `[33m`, `[0m`, `[97m`) and
/// `[<params>K` (erase-line, e.g. `[2K`, or bare `[K`). Mirrors the regex
/// `\[[0-9;]*m` (and the `K`-terminated equivalent): a `[` followed by zero
/// or more digits/semicolons and terminated by `m` or `K` is treated as a
/// stray escape sequence. Deliberately narrow to that shape so a literal `[`
/// in real log text (file paths, array indices, log-level brackets like
/// `[INFO]`) is left untouched — plain `[` followed by anything else, or by
/// digits that never reach an `m`/`K` terminator, passes through unchanged.
fn strip_bare_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '[' {
            out.push(c);
            continue;
        }
        let mut lookahead = chars.clone();
        let mut terminated = false;
        for c2 in lookahead.by_ref() {
            if c2.is_ascii_digit() || c2 == ';' {
                continue;
            }
            terminated = c2 == 'm' || c2 == 'K';
            break;
        }
        if terminated {
            chars = lookahead;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sgr_color_codes() {
        assert_eq!(strip_ansi("\x1b[32mgreen\x1b[0m text"), "green text");
        assert_eq!(strip_ansi("\x1b[1;31mbold red\x1b[0m"), "bold red");
    }

    #[test]
    fn strips_cursor_and_erase_sequences() {
        assert_eq!(strip_ansi("line1\x1b[2Kline2"), "line1line2");
        assert_eq!(strip_ansi("a\x1b[1Ab"), "ab");
        assert_eq!(strip_ansi("\x1b[2Jcleared"), "cleared");
    }

    #[test]
    fn passes_plain_text_through_unchanged() {
        let plain = "2026-07-17T12:00:00 INFO daemon started\nline two";
        assert_eq!(strip_ansi(plain), plain);
    }

    #[test]
    fn handles_multiple_sequences_in_one_line() {
        assert_eq!(
            strip_ansi("\x1b[31merror:\x1b[0m \x1b[2msomething broke\x1b[0m"),
            "error: something broke"
        );
    }

    // ── Defect: bare (ESC-dropped) CSI residue ──────────────────────────
    // Some transports (observed on the daemon's dev-log bridge) drop the raw
    // ESC byte but leave the rest of the SGR sequence behind, so
    // `strip_ansi` must also catch these even with no ESC in sight.

    #[test]
    fn strips_bare_sgr_residue_real_sample() {
        assert_eq!(
            strip_ansi("[33m WARN[97m qt.svg.draw[0m: something happened"),
            " WARN qt.svg.draw: something happened"
        );
    }

    #[test]
    fn strips_bare_sgr_residue_various_codes() {
        assert_eq!(strip_ansi("[1;31mbold red[0m"), "bold red");
        assert_eq!(strip_ansi("[97mwhite[0m"), "white");
    }

    #[test]
    fn strips_bare_erase_line_residue() {
        assert_eq!(strip_ansi("line1[2Kline2"), "line1line2");
        assert_eq!(strip_ansi("bare[Kerase"), "bareerase");
    }

    #[test]
    fn leaves_literal_brackets_untouched() {
        let plain = "[INFO] path[0] = /etc/foo config[bar] done";
        assert_eq!(strip_ansi(plain), plain);
    }

    #[test]
    fn combined_esc_and_bare_residue_in_one_line() {
        // A line that mixes a properly ESC-prefixed sequence with bare
        // residue from a different hop of the pipeline.
        assert_eq!(
            strip_ansi("\x1b[32mok\x1b[0m [33mwarn[0m plain"),
            "ok warn plain"
        );
    }
}
