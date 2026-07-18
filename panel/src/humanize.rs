//! Turns raw daemon reply tokens into plain, human-readable UI. Kept as its
//! own small module (rather than folded into `pages::dashboard` or
//! `pages::controllers`) since the exact same `status`-family token shows up
//! verbatim on both the Dashboard tile and the Controllers fleet section —
//! genuinely shared display logic, unlike the page-local `esc`/`pretty_json`
//! helpers each page intentionally keeps its own copy of.

/// A human-readable rendering of a `status`-family reply
/// (`<connection>:<grab>`, e.g. `connected:grabbed` —
/// `docs/IPC_PROTOCOL.md` § `status`).
pub struct HumanStatus {
    /// CSS class for the small colored state dot (`dot-ok` or `dot-warn`).
    pub dot_class: &'static str,
    /// Plain-language label, e.g. `"Connected · grabbed"`.
    pub label: String,
    /// The original raw token, always exposed too (muted suffix / title
    /// attribute) so the underlying protocol value stays debuggable.
    pub raw: String,
}

/// Parse a `status`/fleet-status reply of the exact shape
/// `<connection>:<grab>` into a [`HumanStatus`]. Returns `None` for anything
/// that isn't that shape (e.g. an already-rendered error message), so
/// callers fall back to showing the raw text unmodified.
pub fn humanize_status(raw: &str) -> Option<HumanStatus> {
    let (connection, grab) = raw.split_once(':')?;
    let conn_label = match connection {
        "connected" => "Connected",
        "disconnected" => "No controllers connected",
        _ => return None,
    };
    // "grabbed" is the shell's normal resting posture (see
    // `controllers.html`'s doc copy) regardless of whether a pad is
    // currently plugged in, so the dot tracks grab state, not connection —
    // connection only changes the wording.
    let grab_label = match (connection, grab) {
        ("connected", "grabbed") => "grabbed",
        ("disconnected", "grabbed") => "grab armed",
        (_, "released") => "released",
        _ => return None,
    };
    let dot_class = if grab == "grabbed" {
        "dot-ok"
    } else {
        "dot-warn"
    };
    Some(HumanStatus {
        dot_class,
        label: format!("{conn_label} · {grab_label}"),
        raw: raw.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_grabbed_is_the_healthy_case() {
        let h = humanize_status("connected:grabbed").unwrap();
        assert_eq!(h.dot_class, "dot-ok");
        assert_eq!(h.label, "Connected · grabbed");
        assert_eq!(h.raw, "connected:grabbed");
    }

    #[test]
    fn disconnected_grabbed_reads_as_grab_armed() {
        let h = humanize_status("disconnected:grabbed").unwrap();
        assert_eq!(h.dot_class, "dot-ok");
        assert_eq!(h.label, "No controllers connected · grab armed");
    }

    #[test]
    fn connected_released_is_a_warn_state() {
        let h = humanize_status("connected:released").unwrap();
        assert_eq!(h.dot_class, "dot-warn");
        assert_eq!(h.label, "Connected · released");
    }

    #[test]
    fn disconnected_released_is_a_warn_state() {
        let h = humanize_status("disconnected:released").unwrap();
        assert_eq!(h.dot_class, "dot-warn");
        assert_eq!(h.label, "No controllers connected · released");
    }

    #[test]
    fn unrecognized_shape_returns_none() {
        assert!(humanize_status("daemon unreachable").is_none());
        assert!(humanize_status("connected:unknown").is_none());
        assert!(humanize_status("no-colon-here").is_none());
    }
}
