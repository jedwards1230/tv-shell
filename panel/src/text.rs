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
}
