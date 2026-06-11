//! Moonlight local-config "forget" — creds-free client-side unpair.
//!
//! Removes a host from Moonlight's local config so THIS client is no longer
//! paired with it. Flipping the host back to "not paired" brings the Pair action
//! back in the shell; re-pairing re-establishes it. Unlike the (removed)
//! Sunshine-side unpair, this needs NO admin credentials — it only edits a local
//! file the user already owns.
//!
//! **Storage** (confirmed on-device): Moonlight (moonlight-qt) stores hosts in
//! `${XDG_CONFIG_HOME:-$HOME/.config}/Moonlight Game Streaming Project/Moonlight.conf`
//! (note the spaces in the directory name). It is a QSettings INI file with a
//! `[hosts]` section containing 1-indexed array groups — `N\hostname`,
//! `N\localaddress`, `N\manualaddress`, `N\remoteaddress`, `N\uuid`,
//! `N\srvcert=@ByteArray(...)`, `N\apps\...`, etc. — plus a section-level
//! `size=N` giving the host count. QSettings `beginReadArray` reads indices
//! `1..size`, so the groups MUST stay contiguous `1..k`; a gap breaks the read.
//!
//! **Approach:** operate **line-based** and preserve every byte we don't touch.
//! We do NOT round-trip through a QSettings serializer — that would reformat,
//! re-quote, and possibly corrupt the `@ByteArray(...)` srvcert blobs. The pure
//! core ([`forget_host`]) takes the conf text and returns the rewritten text;
//! the IPC handler just does read → `forget_host` → write.
//!
//! **Concurrency caveat:** moonlight-qt is invoked once per command by the shell
//! (`moonlight stream/list/pair …`), not held as a persistent process, so there
//! is no live process whose in-memory QSettings would clobber our edit between
//! invocations. Editing the file while no moonlight process is running is safe.

use crate::protocol;
use std::path::PathBuf;

/// The host-array keys whose value identifies a host by the address/name the
/// shell uses. A `moonlight-forget <host>` matches if ANY of these equals
/// `<host>` for some array index.
const MATCH_KEYS: [&str; 4] = ["hostname", "localaddress", "manualaddress", "remoteaddress"];

/// Default Moonlight config path:
/// `${XDG_CONFIG_HOME:-$HOME/.config}/Moonlight Game Streaming Project/Moonlight.conf`.
pub fn moonlight_conf_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".config")
        });
    base.join("Moonlight Game Streaming Project")
        .join("Moonlight.conf")
}

/// Whether `line` belongs to the `[hosts]` array at index `idx` — i.e. it starts
/// with `"<idx>\\"` (QSettings uses a backslash to separate the array index from
/// the key). `idx` is the 1-based array position.
fn line_in_host_group(line: &str, idx: usize) -> bool {
    let trimmed = line.trim_start();
    let prefix = format!("{idx}\\");
    trimmed.starts_with(&prefix)
}

/// Parse the `key=value` of a `[hosts]` array line `"<idx>\\key=value"`, returning
/// `(key_without_index, value)`. Returns `None` if the line is not an
/// index-prefixed `key=value` for `idx`. The key may itself contain backslashes
/// (e.g. `apps\1\appname`) — we only strip the leading `"<idx>\\"`.
fn host_line_kv(line: &str, idx: usize) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix(&format!("{idx}\\"))?;
    let eq = rest.find('=')?;
    Some((&rest[..eq], &rest[eq + 1..]))
}

/// Rewrite a Moonlight `Moonlight.conf` text so that the host matching `host`
/// (by hostname/localaddress/manualaddress/remoteaddress) is removed from the
/// `[hosts]` array, the remaining hosts are renumbered contiguously `1..k`, and
/// the section `size=k` is updated. Everything outside the matched host's lines
/// is preserved verbatim (other sections, other hosts' keys, srvcert blobs).
///
/// Idempotent: if no host matches (or there is no `[hosts]` section), the input
/// is returned unchanged. Pure — no I/O — so it is unit-tested cross-platform.
pub fn forget_host(conf_text: &str, host: &str) -> String {
    let lines: Vec<&str> = conf_text.lines().collect();

    // 1. Locate the [hosts] section's line range [start_body, end) where
    //    start_body is the first line AFTER the `[hosts]` header and `end` is the
    //    next section header (or EOF).
    let mut hosts_header: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim() == "[hosts]" {
            hosts_header = Some(i);
            break;
        }
    }
    let Some(header_idx) = hosts_header else {
        return conf_text.to_string(); // no [hosts] section — nothing to forget.
    };
    let body_start = header_idx + 1;
    let mut body_end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(body_start) {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            body_end = i;
            break;
        }
    }

    // 2. Determine the current host count from `size=N` (default 0). Find the
    //    matching array index, if any.
    let mut size: usize = 0;
    for line in &lines[body_start..body_end] {
        if let Some(v) = line.trim().strip_prefix("size=") {
            if let Ok(n) = v.trim().parse::<usize>() {
                size = n;
            }
        }
    }

    let mut matched: Option<usize> = None;
    'outer: for idx in 1..=size {
        for line in &lines[body_start..body_end] {
            if let Some((key, value)) = host_line_kv(line, idx) {
                if MATCH_KEYS.contains(&key) && value == host {
                    matched = Some(idx);
                    break 'outer;
                }
            }
        }
    }

    let Some(matched_idx) = matched else {
        return conf_text.to_string(); // host not found — already forgotten.
    };

    // 3. Rebuild the file. Within the [hosts] body: drop the matched index's
    //    lines, renumber every higher index down by one (so the array stays
    //    contiguous), rewrite `size=` to the new count, and keep all other body
    //    lines (e.g. a stray comment) verbatim. Outside the body: copy verbatim.
    let new_size = size - 1;
    let mut out: Vec<String> = Vec::with_capacity(lines.len());

    for (i, line) in lines.iter().enumerate() {
        let in_body = (body_start..body_end).contains(&i);
        if !in_body {
            out.push((*line).to_string());
            continue;
        }

        // The section-level size line → updated count.
        if line.trim().starts_with("size=") {
            out.push(format!("size={new_size}"));
            continue;
        }

        // A line belonging to the matched host → drop it.
        if line_in_host_group(line, matched_idx) {
            continue;
        }

        // A line belonging to a higher-index host → renumber index-1.
        // Find which index this line belongs to (if any) by checking 1..=size.
        let mut renumbered: Option<String> = None;
        for idx in (matched_idx + 1)..=size {
            if line_in_host_group(line, idx) {
                // Replace only the leading "<idx>\\" with "<idx-1>\\"; the rest
                // (key + '=' + value, incl. any backslashes/ByteArray) is verbatim.
                let trimmed_start = line.len() - line.trim_start().len();
                let indent = &line[..trimmed_start];
                let rest = line.trim_start();
                let suffix = &rest[format!("{idx}\\").len()..];
                renumbered = Some(format!("{indent}{}\\{suffix}", idx - 1));
                break;
            }
        }
        match renumbered {
            Some(r) => out.push(r),
            None => out.push((*line).to_string()),
        }
    }

    // Preserve a trailing newline iff the input had one (lines() strips it).
    let mut result = out.join("\n");
    if conf_text.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// IPC handler for `moonlight-forget <host>`: read the conf, forget the host,
/// write it back. Idempotent — a missing conf or an unknown host returns `ok`.
/// Runs in the blocking pool (see ipc.rs) since it does sync file I/O.
pub fn handle_forget(host: &str) -> String {
    let path = moonlight_conf_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        // No conf yet → nothing to forget (idempotent success).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return protocol::resp_ok(),
        Err(e) => return protocol::resp_error(&protocol::sanitize_ipc(&e.to_string())),
    };

    let rewritten = forget_host(&text, host);

    // No change (host already absent) → success without a needless write.
    if rewritten == text {
        return protocol::resp_ok();
    }

    match std::fs::write(&path, rewritten) {
        Ok(()) => protocol::resp_ok(),
        Err(e) => protocol::resp_error(&protocol::sanitize_ipc(&e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A realistic two-host Moonlight.conf with other sections and srvcert blobs.
    // Host 1 = desktop-1, host 2 = laptop. Note the spaces-in-dir is irrelevant
    // here (we parse text, not the path). The srvcert ByteArray is shortened but
    // shaped like the real `@ByteArray(...)` value, which must survive verbatim.
    const TWO_HOSTS: &str = "[General]\n\
theme=dark\n\
\n\
[hosts]\n\
1\\hostname=desktop-1\n\
1\\localaddress=192.168.8.10\n\
1\\manualaddress=\n\
1\\remoteaddress=\n\
1\\uuid=uuid-one\n\
1\\srvcert=@ByteArray(-----BEGIN CERT one-----)\n\
2\\hostname=laptop\n\
2\\localaddress=192.168.8.20\n\
2\\manualaddress=\n\
2\\remoteaddress=\n\
2\\uuid=uuid-two\n\
2\\srvcert=@ByteArray(-----BEGIN CERT two-----)\n\
size=2\n\
\n\
[gcmapping]\n\
0030000058420000=mapping-blob\n";

    const ONE_HOST: &str = "[hosts]\n\
1\\hostname=desktop-1\n\
1\\localaddress=192.168.8.10\n\
1\\uuid=uuid-one\n\
1\\srvcert=@ByteArray(blob)\n\
size=1\n";

    #[test]
    fn forget_first_of_two_renumbers_second_to_first() {
        let out = forget_host(TWO_HOSTS, "desktop-1");
        // desktop-1's lines are gone.
        assert!(!out.contains("desktop-1"));
        assert!(!out.contains("CERT one"));
        // laptop survives and is renumbered 2 -> 1.
        assert!(out.contains("1\\hostname=laptop"));
        assert!(out.contains("1\\localaddress=192.168.8.20"));
        assert!(out.contains("1\\uuid=uuid-two"));
        // laptop's srvcert ByteArray is preserved verbatim under the new index.
        assert!(out.contains("1\\srvcert=@ByteArray(-----BEGIN CERT two-----)"));
        // No stale index-2 lines remain.
        assert!(!out.contains("2\\hostname"));
        // size decremented.
        assert!(out.contains("size=1"));
        // Other sections preserved verbatim.
        assert!(out.contains("[General]"));
        assert!(out.contains("theme=dark"));
        assert!(out.contains("[gcmapping]"));
        assert!(out.contains("0030000058420000=mapping-blob"));
    }

    #[test]
    fn forget_match_by_localaddress() {
        // Match by an address field, not just hostname.
        let out = forget_host(TWO_HOSTS, "192.168.8.10");
        assert!(!out.contains("desktop-1"));
        assert!(out.contains("1\\hostname=laptop"));
        assert!(out.contains("size=1"));
    }

    #[test]
    fn forget_second_of_two_leaves_first_untouched() {
        let out = forget_host(TWO_HOSTS, "laptop");
        // laptop gone.
        assert!(!out.contains("laptop"));
        assert!(!out.contains("CERT two"));
        // desktop-1 stays at index 1, unchanged.
        assert!(out.contains("1\\hostname=desktop-1"));
        assert!(out.contains("1\\srvcert=@ByteArray(-----BEGIN CERT one-----)"));
        assert!(!out.contains("2\\hostname"));
        assert!(out.contains("size=1"));
    }

    #[test]
    fn forget_nonmatching_host_is_unchanged() {
        let out = forget_host(TWO_HOSTS, "no-such-host");
        assert_eq!(out, TWO_HOSTS);
    }

    #[test]
    fn forget_only_host_leaves_empty_array() {
        let out = forget_host(ONE_HOST, "desktop-1");
        // [hosts] section remains, but with no N\ lines and size=0.
        assert!(out.contains("[hosts]"));
        assert!(out.contains("size=0"));
        assert!(!out.contains("desktop-1"));
        assert!(!out.contains("1\\hostname"));
        assert!(!out.contains("@ByteArray"));
    }

    #[test]
    fn no_hosts_section_is_unchanged() {
        let conf = "[General]\ntheme=dark\n";
        assert_eq!(forget_host(conf, "desktop-1"), conf);
    }

    #[test]
    fn trailing_newline_preserved_or_absent() {
        // With trailing newline.
        let with_nl = ONE_HOST; // ends with \n
        assert!(forget_host(with_nl, "nope").ends_with('\n'));
        // Without trailing newline.
        let without_nl = "[hosts]\n1\\hostname=h\nsize=1";
        let out = forget_host(without_nl, "h");
        assert!(!out.ends_with('\n'));
        assert!(out.contains("size=0"));
    }
}
