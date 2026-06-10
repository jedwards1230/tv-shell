//! Notification history tracking (`~/.local/share/game-shell/notifications.json`).
//!
//! Persists the notification history shown in `components/NotificationManager.qml`
//! so the notification center survives Quickshell restarts. Exposed over IPC as
//! `record-notification`, `set-notifications`, and `get-notifications` (see
//! `docs/IPC_PROTOCOL.md`).
//!
//! Platform-independent: the file format logic is pure read-modify-write over
//! strings (like `recents.rs`), so it is unit-tested on any host. Only the thin
//! `record_notification`/`set_notifications`/`load_notifications` wrappers touch
//! the filesystem.
//!
//! Behavior:
//!   - newest entry first (PREPEND); NO de-dup — every notification is a distinct
//!     log event (unlike recents which deduplicates by name)
//!   - the stored file is capped at `MAX_ENTRIES` (100) entries
//!   - written single-line compact JSON (QML's `SplitParser` reads line-by-line)
//!   - each entry is `{id, title, message, level, source, icon, time}` where
//!     `time` is unix seconds

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Cap on stored entries.
pub const MAX_ENTRIES: usize = 100;

/// One notification history entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub icon: String,
    /// Unix seconds (float, matching Python `time.time()`).
    #[serde(default)]
    pub time: f64,
}

/// Default notifications path: `~/.local/share/game-shell/notifications.json`.
pub fn notifications_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".local/share/game-shell/notifications.json")
}

/// Parse the notifications file contents into a list. Invalid/missing JSON yields
/// an empty list (graceful — same as recents).
pub fn parse_notifications(contents: &str) -> Vec<Notification> {
    serde_json::from_str(contents).unwrap_or_default()
}

/// Prepend a new notification onto the existing list and cap at [`MAX_ENTRIES`].
/// Does NOT de-duplicate — every notification is a distinct log event.
/// Returns the new list (caller serializes it). Pure — `entry.time` is injected
/// by the caller so the timestamp is testable.
pub fn record(existing: Vec<Notification>, entry: Notification) -> Vec<Notification> {
    let mut out = existing;
    out.insert(0, entry);
    out.truncate(MAX_ENTRIES);
    out
}

/// Overwrite the list entirely with `entries`, capping at [`MAX_ENTRIES`]. Used
/// for clears and individual removals (full-array overwrite from QML state).
pub fn set_all(entries: Vec<Notification>) -> Vec<Notification> {
    let mut out = entries;
    out.truncate(MAX_ENTRIES);
    out
}

/// Serialize notifications as compact single-line JSON (QML requires single-line).
/// Serialization of these plain structs is infallible, but degrade to `[]`
/// rather than panic the daemon on the impossible case.
pub fn notifications_to_json(notifications: &[Notification]) -> String {
    serde_json::to_string(notifications).unwrap_or_else(|_| "[]".to_string())
}

/// Atomically write `contents` to `path` (write a sibling temp file, then
/// rename over the target) so a crash mid-write can't leave a torn/corrupt
/// file. QML is the sole, serial writer, so lost-update races aren't reachable
/// in practice; this guards against partial writes.
fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

/// Read the notifications file and return all stored entries (up to
/// [`MAX_ENTRIES`]) as a compact JSON array (the `get-notifications` response
/// body). Missing/invalid file -> `[]`.
pub fn load_notifications(path: &Path) -> String {
    let notifications = std::fs::read_to_string(path)
        .ok()
        .map(|t| parse_notifications(&t))
        .unwrap_or_default();
    notifications_to_json(&notifications)
}

/// Record a notification: read existing, prepend, cap, and write single-line.
/// The timestamp is set by the caller (unix seconds).
pub fn record_notification(path: &Path, entry: Notification) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path)
        .ok()
        .map(|t| parse_notifications(&t))
        .unwrap_or_default();
    let updated = record(existing, entry);
    atomic_write(path, &notifications_to_json(&updated))
}

/// Overwrite the notifications file with the given list (capped at
/// [`MAX_ENTRIES`]). Used for clears and removals.
pub fn set_notifications(path: &Path, entries: Vec<Notification>) -> std::io::Result<()> {
    let updated = set_all(entries);
    atomic_write(path, &notifications_to_json(&updated))
}

/// Current wall-clock time in unix seconds (float), like Python `time.time()`.
pub fn now_unix_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: i64, title: &str, time: f64) -> Notification {
        Notification {
            id,
            title: title.into(),
            message: String::new(),
            level: "info".into(),
            source: "system".into(),
            icon: String::new(),
            time,
        }
    }

    #[test]
    fn parse_missing_or_invalid_is_empty() {
        assert_eq!(parse_notifications(""), vec![]);
        assert_eq!(parse_notifications("not json"), vec![]);
        assert_eq!(parse_notifications("{}"), vec![]);
    }

    #[test]
    fn parse_round_trips_entries() {
        let json = r#"[{"id":1,"title":"Test","message":"msg","level":"info","source":"system","icon":"","time":1.5}]"#;
        let parsed = parse_notifications(json);
        assert_eq!(
            parsed,
            vec![Notification {
                id: 1,
                title: "Test".into(),
                message: "msg".into(),
                level: "info".into(),
                source: "system".into(),
                icon: String::new(),
                time: 1.5,
            }]
        );
    }

    #[test]
    fn record_prepends_without_dedup() {
        let existing = vec![n(1, "A", 1.0), n(2, "B", 2.0)];
        // Adding another entry with title "A" — it is NOT de-duped (unlike recents).
        let out = record(existing, n(3, "A", 9.0));
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id, 3);
        assert_eq!(out[0].time, 9.0);
        assert_eq!(out[1].id, 1);
        assert_eq!(out[2].id, 2);
    }

    #[test]
    fn record_caps_at_max_entries() {
        let existing: Vec<Notification> = (0..MAX_ENTRIES as i64)
            .map(|i| n(i, &format!("Notif{i}"), i as f64))
            .collect();
        let out = record(existing, n(MAX_ENTRIES as i64, "New", 100.0));
        assert_eq!(out.len(), MAX_ENTRIES);
        assert_eq!(out[0].title, "New");
        // The oldest fell off the end.
        assert!(!out.iter().any(|e| e.title == format!("Notif{}", MAX_ENTRIES - 1)));
    }

    #[test]
    fn set_all_overwrites_and_caps() {
        // More than MAX_ENTRIES: truncated.
        let many: Vec<Notification> = (0..MAX_ENTRIES as i64 + 10)
            .map(|i| n(i, &format!("N{i}"), i as f64))
            .collect();
        let out = set_all(many);
        assert_eq!(out.len(), MAX_ENTRIES);
        // Replacing with a small list works too.
        let small = vec![n(1, "A", 1.0), n(2, "B", 2.0)];
        let out2 = set_all(small.clone());
        assert_eq!(out2, small);
        // Empty list clears everything.
        let out3 = set_all(vec![]);
        assert_eq!(out3, vec![]);
    }

    #[test]
    fn record_notification_then_load_round_trips_on_disk() {
        let path = std::env::temp_dir().join(format!(
            "gs-notifications-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);

        // Missing file -> empty array.
        assert_eq!(load_notifications(&path), "[]");

        record_notification(&path, n(1, "Alert", 1.0)).unwrap();
        record_notification(&path, n(2, "Warning", 2.0)).unwrap();
        // A second notification with the same title is recorded separately (no dedup).
        record_notification(&path, n(3, "Alert", 3.0)).unwrap();

        let loaded = parse_notifications(&load_notifications(&path));
        assert_eq!(loaded.len(), 3);
        // Newest first.
        assert_eq!(loaded[0].id, 3);
        assert_eq!(loaded[1].id, 2);
        assert_eq!(loaded[2].id, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn set_notifications_overwrites_on_disk() {
        let path = std::env::temp_dir().join(format!(
            "gs-notifications-set-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);

        record_notification(&path, n(1, "A", 1.0)).unwrap();
        record_notification(&path, n(2, "B", 2.0)).unwrap();

        // Overwrite with a single entry (simulates removeFromHistory).
        set_notifications(&path, vec![n(1, "A", 1.0)]).unwrap();
        let loaded = parse_notifications(&load_notifications(&path));
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, 1);

        // Clear everything.
        set_notifications(&path, vec![]).unwrap();
        assert_eq!(load_notifications(&path), "[]");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn json_is_compact_single_line() {
        let json = notifications_to_json(&[n(1, "X", 1.0)]);
        assert!(!json.contains('\n'));
        assert!(!json.contains(": "));
        assert_eq!(notifications_to_json(&[]), "[]");
    }
}
