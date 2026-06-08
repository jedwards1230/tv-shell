//! System and storage status IPC for the System/About settings page (#164).
//!
//! Both commands are purely stateless reads from procfs/sysfs and `/etc`:
//!
//! - `sys-status`     → `{"os":"…","kernel":"…","hostname":"…","uptime":"…"}`
//! - `storage-status` → `[{"mount":"…","size":…,"used":…,"avail":…,"pct":…}, …]`
//!
//! These are cross-platform (no Linux-only imports — they degrade gracefully on
//! non-Linux) and are served directly from `dispatch_stateless` without a round-
//! trip to the input runtime.

use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// sys-status
// ---------------------------------------------------------------------------

/// Read the `NAME=` field from `/etc/os-release`. Returns `"Unknown"` on
/// any error (file absent, no match, encoding issues).
fn os_name() -> String {
    let content = fs::read_to_string("/etc/os-release").unwrap_or_default();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("NAME=") {
            return rest.trim_matches('"').to_string();
        }
    }
    "Unknown".to_string()
}

/// Read the kernel release string from `/proc/sys/kernel/osrelease`
/// (equivalent to `uname -r`).
fn kernel_version() -> String {
    fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|_| "Unknown".into())
        .trim()
        .to_string()
}

/// Read the hostname from `/proc/sys/kernel/hostname`.
fn hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap_or_else(|_| "Unknown".into())
        .trim()
        .to_string()
}

/// Format an uptime in seconds as `Xd Xh Xm Xs`, eliding leading zero units
/// (e.g. `90` -> `"1m 30s"`, `2d 3h 5m 10s` for a multi-day uptime).
fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m {seconds}s")
    } else if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

/// Read system uptime from `/proc/uptime` and format it as `Xd Xh Xm Xs`.
fn uptime_string() -> String {
    let raw = fs::read_to_string("/proc/uptime").unwrap_or_default();
    let secs = raw
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0);
    format_uptime(secs)
}

/// Build the `sys-status` JSON response.
pub fn sys_status_json() -> String {
    let os = serde_json::Value::String(os_name());
    let kernel = serde_json::Value::String(kernel_version());
    let host = serde_json::Value::String(hostname());
    let uptime = serde_json::Value::String(uptime_string());
    // Construct the object manually to guarantee field order and avoid an extra struct.
    format!(
        r#"{{"os":{},"kernel":{},"hostname":{},"uptime":{}}}"#,
        os, kernel, host, uptime
    )
}

// ---------------------------------------------------------------------------
// storage-status
// ---------------------------------------------------------------------------

/// One filesystem mount entry for `storage-status`.
#[derive(Debug, serde::Serialize)]
pub struct MountEntry {
    pub mount: String,
    /// Total size in bytes.
    pub size: u64,
    /// Used bytes.
    pub used: u64,
    /// Available (free) bytes.
    pub avail: u64,
    /// Usage percentage 0..=100.
    pub pct: u8,
}

/// Parse `/proc/mounts` and collect real filesystem mount points, then read
/// statvfs for each to get usage. Skips pseudo-filesystems (procfs, sysfs,
/// devtmpfs, tmpfs on /run/user/*, cgroup, etc.) using a small deny-list of
/// fs-types and mount-point prefixes that are never interesting to users.
///
/// This mirrors what `df -h` shows but returns raw bytes — the QML layer can
/// format them however it likes.
pub fn storage_status_json() -> String {
    let mounts = collect_mounts();
    serde_json::to_string(&mounts).unwrap_or_else(|_| "[]".into())
}

fn collect_mounts() -> Vec<MountEntry> {
    let content = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut seen_devices: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut entries: Vec<MountEntry> = Vec::new();

    for line in content.lines() {
        let parts: Vec<&str> = line.splitn(6, ' ').collect();
        if parts.len() < 4 {
            continue;
        }
        let device = parts[0];
        let mountpoint = parts[1];
        let fstype = parts[2];

        // Skip pseudo / kernel / user-specific filesystems.
        if is_skipped_fstype(fstype) || is_skipped_mountpoint(mountpoint) {
            continue;
        }

        // De-duplicate by device (e.g. the same disk mounted for bind in multiple places).
        if device.starts_with('/') && !seen_devices.insert(device.to_string()) {
            continue;
        }

        if let Some(entry) = statvfs_entry(mountpoint) {
            entries.push(entry);
        }
    }
    entries
}

fn is_skipped_fstype(fstype: &str) -> bool {
    matches!(
        fstype,
        "proc"
            | "sysfs"
            | "devtmpfs"
            | "devpts"
            | "tmpfs"
            | "cgroup"
            | "cgroup2"
            | "pstore"
            | "bpf"
            | "tracefs"
            | "debugfs"
            | "securityfs"
            | "selinuxfs"
            | "fusectl"
            | "efivarfs"
            | "mqueue"
            | "hugetlbfs"
            | "nsfs"
            | "ramfs"
            | "overlay"
            | "aufs"
    )
}

fn is_skipped_mountpoint(mountpoint: &str) -> bool {
    mountpoint.starts_with("/proc")
        || mountpoint.starts_with("/sys")
        || mountpoint.starts_with("/dev/pts")
        || mountpoint.starts_with("/run/user")
        || mountpoint.starts_with("/run/snapd")
        || mountpoint == "/dev"
}

/// Call `statvfs(2)` on `mountpoint` and return a `MountEntry` with raw-byte
/// sizes. Returns `None` if the call fails (mount disappeared mid-read, etc.).
fn statvfs_entry(mountpoint: &str) -> Option<MountEntry> {
    use std::mem::MaybeUninit;

    let path = std::ffi::CString::new(mountpoint).ok()?;
    let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();

    // SAFETY: path is a valid NUL-terminated string; stat is written before use.
    let rc = unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    // SAFETY: statvfs succeeded, so stat is fully initialised.
    let stat = unsafe { stat.assume_init() };

    // f_bsize/f_blocks/f_bavail/f_bfree are u32 on Linux and u64 on macOS;
    // cast uniformly to u64 so arithmetic compiles on both platforms.
    #[allow(clippy::unnecessary_cast)]
    let bsize = stat.f_bsize as u64;
    #[allow(clippy::unnecessary_cast)]
    let size = stat.f_blocks as u64 * bsize;
    #[allow(clippy::unnecessary_cast)]
    let avail = stat.f_bavail as u64 * bsize;
    #[allow(clippy::unnecessary_cast)]
    let used = size.saturating_sub(stat.f_bfree as u64 * bsize);
    let pct = if size > 0 {
        ((used as f64 / size as f64) * 100.0).round() as u8
    } else {
        0
    };

    // Skip zero-size mounts (e.g. bind mounts of /dev entries).
    if size == 0 {
        return None;
    }

    // Verify the mountpoint still exists before reporting it.
    if !Path::new(mountpoint).exists() {
        return None;
    }

    Some(MountEntry {
        mount: mountpoint.to_string(),
        size,
        used,
        avail,
        pct,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_formats_seconds_only() {
        // 90 seconds = 1m 30s — exercises the real formatter.
        assert_eq!(format_uptime(90), "1m 30s");
    }

    #[test]
    fn uptime_formats_with_days() {
        // 2 days + 3 hours + 5 min + 10 sec — exercises the real formatter.
        let secs = 2 * 86400 + 3 * 3600 + 5 * 60 + 10;
        assert_eq!(format_uptime(secs), "2d 3h 5m 10s");
    }

    #[test]
    fn uptime_elides_zero_units() {
        assert_eq!(format_uptime(0), "0s");
        assert_eq!(format_uptime(45), "45s");
        assert_eq!(format_uptime(3600), "1h 0m 0s");
    }

    #[test]
    fn sys_status_json_is_valid_json_with_required_keys() {
        let json = sys_status_json();
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(v.get("os").is_some(), "missing 'os' field");
        assert!(v.get("kernel").is_some(), "missing 'kernel' field");
        assert!(v.get("hostname").is_some(), "missing 'hostname' field");
        assert!(v.get("uptime").is_some(), "missing 'uptime' field");
    }

    #[test]
    fn skipped_fstypes_match_pseudo_filesystems() {
        assert!(is_skipped_fstype("proc"));
        assert!(is_skipped_fstype("sysfs"));
        assert!(is_skipped_fstype("devtmpfs"));
        assert!(is_skipped_fstype("cgroup2"));
        assert!(
            is_skipped_fstype("tmpfs"),
            "tmpfs must be skipped (covers /dev/shm, /run, /tmp)"
        );
        assert!(!is_skipped_fstype("ext4"));
        assert!(!is_skipped_fstype("btrfs"));
        assert!(!is_skipped_fstype("xfs"));
    }

    #[test]
    fn skipped_mountpoints_match_system_paths() {
        assert!(is_skipped_mountpoint("/proc/self/mnt"));
        assert!(is_skipped_mountpoint("/sys/fs/cgroup"));
        assert!(is_skipped_mountpoint("/run/user/1000"));
        assert!(!is_skipped_mountpoint("/home"));
        assert!(!is_skipped_mountpoint("/mnt/data"));
    }

    #[test]
    fn storage_status_json_is_valid_json_array() {
        // On any host (Linux or macOS) this returns a valid JSON array,
        // even if it's empty (non-Linux hosts won't have /proc/mounts).
        let json = storage_status_json();
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(v.is_array(), "storage-status must be a JSON array");
    }
}
