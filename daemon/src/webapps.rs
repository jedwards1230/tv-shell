//! Web-app registry + XDG `.desktop` generation (#187 phases P1 + P3).
//!
//! A "web app" (YouTube, Netflix, …) is just a Chromium `--app=<url>` launcher
//! published as an XDG desktop entry in `~/.local/share/applications/`. Because
//! the daemon already scans that directory (`apps::scan_apps`), a generated
//! entry flows into `list-apps`, the home Applications row, and
//! `intent app:<wmClass>` **for free** — no new launch plumbing. See
//! [`docs/WEB_APPS.md`](../../docs/WEB_APPS.md) for the design.
//!
//! **The daemon is the sole writer** of both the registry (the `webApps` key in
//! settings.json) and the generated `.desktop` files, mirroring how it solely
//! writes settings.json. QML reads `SettingsStore.webApps` as a read-only
//! mirror; the web control panel drives add/remove over IPC.
//!
//! **Ownership marker.** Every entry we generate carries
//! `X-TvShell-WebApp=true`, and we only ever rewrite/delete files carrying it,
//! so a hand-written or distro-shipped `.desktop` can never be clobbered by a
//! slug collision. The pre-rebrand `X-GameShell-WebApp=true` is still accepted
//! when detecting ownership, matching the header-compatibility precedent in
//! `panel/src/bridge.rs`.
//!
//! Cross-platform: pure path/string/JSON work, so it builds and unit-tests on
//! macOS alongside the daemon's other portable modules (`apps`, `recents`, …).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Desktop-entry key marking an entry as generated and owned by us.
pub const OWNER_KEY: &str = "X-TvShell-WebApp";
/// Pre-rebrand spelling, still honored when detecting ownership.
pub const LEGACY_OWNER_KEY: &str = "X-GameShell-WebApp";

/// Longest generated slug, keeping filenames and window classes sane.
const MAX_SLUG: usize = 32;
/// Defensive caps so a hostile/fat-fingered panel POST can't write a huge entry.
const MAX_NAME: usize = 64;
const MAX_URL: usize = 2048;

/// One registry entry. Field names match the JSON shape fixed by
/// `docs/WEB_APPS.md`: `{ "id", "name", "url", "wmClass" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebApp {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(rename = "wmClass")]
    pub wm_class: String,
}

// ---------------------------------------------------------------------------
// Validation + slugging (pure)
// ---------------------------------------------------------------------------

/// Reject anything that can't appear safely on a single desktop-entry line:
/// control characters (notably newline, which would forge new keys) and DEL.
fn has_control_chars(s: &str) -> bool {
    s.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}')
}

/// Validate a display name: non-empty, single-line, length-capped.
pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name must not be empty".to_string());
    }
    if name.len() > MAX_NAME {
        return Err(format!("name must be at most {MAX_NAME} characters"));
    }
    if has_control_chars(name) {
        return Err("name must not contain control characters or newlines".to_string());
    }
    Ok(name.to_string())
}

/// Validate a launch URL: `http`/`https` only, single-line, no characters that
/// would need shell/desktop-entry quoting games (a real URL percent-encodes
/// them anyway). This is the gate that keeps the generated `Exec=` line honest.
pub fn validate_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("URL must not be empty".to_string());
    }
    if url.len() > MAX_URL {
        return Err(format!("URL must be at most {MAX_URL} characters"));
    }
    if has_control_chars(url) {
        return Err("URL must not contain control characters or newlines".to_string());
    }
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("URL must start with http:// or https://".to_string());
    }
    // Bare host required after the scheme.
    let rest = &url[url.find("//").map(|i| i + 2).unwrap_or(url.len())..];
    if rest.is_empty() || rest.starts_with('/') {
        return Err("URL must include a host, e.g. https://example.com".to_string());
    }
    if url.chars().any(|c| {
        c.is_whitespace() || c == '"' || c == '\'' || c == '\\' || c == '`' || c == '$' || c == ';'
    }) {
        return Err(
            "URL must not contain whitespace, quotes, backslashes, backticks, $ or ;".to_string(),
        );
    }
    Ok(url.to_string())
}

/// Derive a stable slug from a display name: lowercase ASCII alphanumerics,
/// every other run collapsed to a single `-`. Falls back to `"webapp"` when a
/// name has no usable ASCII (e.g. all-emoji), so an id is always producible.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c.to_ascii_lowercase());
            if out.len() >= MAX_SLUG {
                break;
            }
        } else {
            pending_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "webapp".to_string()
    } else {
        out
    }
}

/// Make `base` unique against ids already in the registry by suffixing `-2`,
/// `-3`, … The suffix is applied deterministically, so re-adding after a
/// removal reuses the freed id rather than drifting upward forever.
pub fn unique_id(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    for n in 2..10_000 {
        let candidate = format!("{base}-{n}");
        if !taken.iter().any(|t| t == &candidate) {
            return candidate;
        }
    }
    // Unreachable in practice (9998 collisions on one slug).
    format!("{base}-x")
}

/// The window class we assign, matching Chromium's `--class` and the entry's
/// `StartupWMClass` so the daemon's existing window matching works unchanged.
pub fn wm_class_for(id: &str) -> String {
    format!("tvshell-{id}")
}

/// Generated entry filename — namespaced so it can never collide with a
/// distro-shipped entry.
pub fn desktop_file_name(id: &str) -> String {
    format!("tv-shell-webapp-{id}.desktop")
}

// ---------------------------------------------------------------------------
// Desktop-entry rendering (pure)
// ---------------------------------------------------------------------------

/// Escape a value for a desktop-entry `Exec=` argument. Per the spec, `%` is
/// the field-code introducer and must be doubled; backslashes are escaped too.
/// [`validate_url`]/[`validate_name`] already reject quotes and control
/// characters, so this is defense in depth rather than the only guard.
fn exec_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "%%")
}

/// Render the full `.desktop` contents for a web app.
///
/// `chromium` is the browser binary (absolute path or bare name resolved via
/// PATH). `user_data_dir` is the app's dedicated Chromium profile, so logins
/// and DRM state stay isolated per web app.
pub fn desktop_entry(app: &WebApp, chromium: &str, user_data_dir: &Path) -> String {
    let exec = format!(
        "{} --app={} --class={} --user-data-dir={} --ozone-platform=wayland",
        exec_escape(chromium),
        exec_escape(&app.url),
        exec_escape(&app.wm_class),
        exec_escape(&user_data_dir.display().to_string()),
    );
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         Comment=Web app launched by tv-shell\n\
         Exec={exec}\n\
         Icon=web-browser\n\
         Terminal=false\n\
         Categories=Network;\n\
         StartupWMClass={wm_class}\n\
         {owner}=true\n\
         X-TvShell-WebAppId={id}\n\
         X-TvShell-WebAppUrl={url}\n",
        name = app.name,
        exec = exec,
        wm_class = app.wm_class,
        owner = OWNER_KEY,
        id = app.id,
        url = app.url,
    )
}

/// Is this `.desktop` file one of ours (safe to rewrite/delete)? Accepts the
/// current and pre-rebrand marker keys.
pub fn entry_is_ours(contents: &str) -> bool {
    contents.lines().any(|line| {
        let line = line.trim();
        line.eq_ignore_ascii_case(&format!("{OWNER_KEY}=true"))
            || line.eq_ignore_ascii_case(&format!("{LEGACY_OWNER_KEY}=true"))
    })
}

// ---------------------------------------------------------------------------
// Paths + browser discovery
// ---------------------------------------------------------------------------

/// `~/.local/share/applications` — the per-user XDG entry directory the
/// daemon's own app scan already reads.
pub fn applications_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir).join("applications");
    }
    home_dir().join(".local/share/applications")
}

/// Per-web-app Chromium profile dir, under our own data dir.
pub fn user_data_dir(id: &str) -> PathBuf {
    tv_shell_protocol::brand::data_dir()
        .join("webapps")
        .join(id)
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Chromium-family binaries we accept, in preference order. Web apps need
/// Widevine for Netflix/Plex, which is why this is Chromium rather than the
/// system default browser.
const CHROMIUM_CANDIDATES: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome-stable",
    "google-chrome",
    "brave-browser",
    "microsoft-edge-stable",
];

/// Find an installed Chromium-family browser by scanning `PATH`. `None` means
/// web apps can be registered but won't launch — callers surface that honestly
/// rather than silently generating dead launchers.
pub fn find_chromium() -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for candidate in CHROMIUM_CANDIDATES {
            let full = dir.join(candidate);
            if is_executable(&full) {
                return Some(full.display().to_string());
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

// ---------------------------------------------------------------------------
// Registry I/O (settings.json is written only via config::set_config)
// ---------------------------------------------------------------------------

/// Parse the `webApps` array out of a settings.json document. A missing,
/// non-array, or partly-malformed value yields the entries that DID parse
/// (never an error) — the registry must degrade rather than wedge add/remove.
pub fn parse_registry(settings_json: &str) -> Vec<WebApp> {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(settings_json) else {
        return Vec::new();
    };
    let Some(arr) = doc.get("webApps").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| serde_json::from_value::<WebApp>(v.clone()).ok())
        .collect()
}

/// Read the current registry from `settings_path`.
pub fn list(settings_path: &Path) -> Vec<WebApp> {
    match std::fs::read_to_string(settings_path) {
        Ok(text) => parse_registry(&text),
        Err(_) => Vec::new(),
    }
}

/// Serialize a registry to the single-line JSON the shell requires
/// (`SplitParser` reads settings.json line-by-line — pretty-printing breaks it).
pub fn registry_json(apps: &[WebApp]) -> serde_json::Value {
    serde_json::to_value(apps).unwrap_or(serde_json::Value::Array(Vec::new()))
}

/// Build the entry for a new web app without touching the filesystem — the
/// pure half of [`add`], so id/slug/collision behavior is unit-testable.
pub fn build_entry(name: &str, url: &str, existing: &[WebApp]) -> Result<WebApp, String> {
    let name = validate_name(name)?;
    let url = validate_url(url)?;
    let taken: Vec<String> = existing.iter().map(|a| a.id.clone()).collect();
    let id = unique_id(&slugify(&name), &taken);
    let wm_class = wm_class_for(&id);
    Ok(WebApp {
        id,
        name,
        url,
        wm_class,
    })
}

/// Write (or overwrite) the `.desktop` file for `app`. Refuses to overwrite a
/// file that exists but is NOT ours, so a slug can never clobber a foreign
/// entry.
pub fn write_desktop_entry(app: &WebApp, chromium: &str, dir: &Path) -> Result<PathBuf, String> {
    let path = dir.join(desktop_file_name(&app.id));
    if let Ok(existing) = std::fs::read_to_string(&path) {
        if !entry_is_ours(&existing) {
            return Err(format!(
                "refusing to overwrite {}: not a tv-shell-generated entry",
                path.display()
            ));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let contents = desktop_entry(app, chromium, &user_data_dir(&app.id));
    crate::config::atomic_write(&path, contents)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(path)
}

/// Delete a web app's `.desktop` file. A missing file is success (idempotent);
/// a file that isn't ours is refused.
pub fn remove_desktop_entry(id: &str, dir: &Path) -> Result<(), String> {
    let path = dir.join(desktop_file_name(id));
    match std::fs::read_to_string(&path) {
        Err(_) => Ok(()), // already gone
        Ok(existing) => {
            if !entry_is_ours(&existing) {
                return Err(format!(
                    "refusing to delete {}: not a tv-shell-generated entry",
                    path.display()
                ));
            }
            std::fs::remove_file(&path)
                .map_err(|e| format!("could not delete {}: {e}", path.display()))
        }
    }
}

/// Add a web app: validate, allocate an id, write the launcher, then persist
/// the registry through `config::set_config` (the single settings.json writer).
///
/// The per-app Chromium profile dir is NOT pre-created — Chromium creates it on
/// first launch — and is deliberately left behind on [`remove`] so a
/// re-added app keeps its logins.
pub fn add(settings_path: &Path, name: &str, url: &str) -> Result<WebApp, String> {
    let mut apps = list(settings_path);
    let app = build_entry(name, url, &apps)?;
    let chromium = find_chromium().ok_or_else(|| {
        "no Chromium-family browser found on PATH (install chromium to launch web apps)".to_string()
    })?;
    write_desktop_entry(&app, &chromium, &applications_dir())?;
    apps.push(app.clone());
    persist(settings_path, &apps)?;
    Ok(app)
}

/// Remove a web app by id: drop it from the registry and delete its launcher.
/// Unknown id is an error (so the panel can report it) rather than a silent ok.
pub fn remove(settings_path: &Path, id: &str) -> Result<(), String> {
    let apps = list(settings_path);
    if !apps.iter().any(|a| a.id == id) {
        return Err(format!("no web app with id {id:?}"));
    }
    let remaining: Vec<WebApp> = apps.into_iter().filter(|a| a.id != id).collect();
    remove_desktop_entry(id, &applications_dir())?;
    persist(settings_path, &remaining)
}

fn persist(settings_path: &Path, apps: &[WebApp]) -> Result<(), String> {
    let updates = serde_json::json!({ "webApps": registry_json(apps) });
    crate::config::set_config(settings_path, &updates)
        .map(|_| ())
        .map_err(|e| format!("could not persist the web-app registry: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_handles_punctuation_and_unicode() {
        assert_eq!(slugify("YouTube"), "youtube");
        assert_eq!(slugify("Plex  TV!"), "plex-tv");
        assert_eq!(slugify("  --Netflix--  "), "netflix");
        assert_eq!(slugify("Disney+"), "disney");
        // No usable ASCII at all still yields a usable id.
        assert_eq!(slugify("🎬🎬"), "webapp");
        assert!(slugify(&"x".repeat(100)).len() <= MAX_SLUG);
    }

    #[test]
    fn unique_id_suffixes_only_on_collision() {
        let taken = vec!["youtube".to_string(), "youtube-2".to_string()];
        assert_eq!(unique_id("plex", &taken), "plex");
        assert_eq!(unique_id("youtube", &taken), "youtube-3");
    }

    #[test]
    fn validate_url_accepts_http_and_rejects_the_rest() {
        assert!(validate_url("https://youtube.com/tv").is_ok());
        assert!(validate_url("http://192.168.1.5:8096").is_ok());
        assert!(validate_url("HTTPS://Example.com").is_ok());
        for bad in [
            "",
            "ftp://example.com",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://",
            "https:///nohost",
            "https://ex ample.com",
            "https://example.com/\"; rm -rf /",
            "https://example.com/`id`",
            "https://example.com/$(id)",
        ] {
            assert!(validate_url(bad).is_err(), "should reject {bad:?}");
        }
        // A newline must never slip through — it would forge desktop-entry keys.
        assert!(validate_url("https://e.com\nExec=/bin/sh").is_err());
    }

    #[test]
    fn validate_name_rejects_newlines_and_empties() {
        assert_eq!(validate_name("  YouTube  ").unwrap(), "YouTube");
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name("Evil\nExec=/bin/sh").is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME + 1)).is_err());
    }

    fn sample() -> WebApp {
        WebApp {
            id: "youtube".to_string(),
            name: "YouTube".to_string(),
            url: "https://youtube.com/tv".to_string(),
            wm_class: "tvshell-youtube".to_string(),
        }
    }

    #[test]
    fn desktop_entry_has_the_contract_keys() {
        let e = desktop_entry(
            &sample(),
            "/usr/bin/chromium",
            Path::new("/home/u/.wa/youtube"),
        );
        assert!(e.starts_with("[Desktop Entry]\n"));
        assert!(e.contains("Name=YouTube\n"));
        assert!(e.contains("StartupWMClass=tvshell-youtube\n"));
        assert!(e.contains("X-TvShell-WebApp=true\n"));
        assert!(e.contains("--app=https://youtube.com/tv"));
        assert!(e.contains("--class=tvshell-youtube"));
        assert!(e.contains("--user-data-dir=/home/u/.wa/youtube"));
        assert!(e.contains("--ozone-platform=wayland"));
        // Exactly one Exec line, and it is single-line.
        assert_eq!(e.lines().filter(|l| l.starts_with("Exec=")).count(), 1);
        assert!(entry_is_ours(&e));
    }

    #[test]
    fn exec_escaping_doubles_percent_signs() {
        let mut app = sample();
        app.url = "https://e.com/a%20b".to_string();
        let e = desktop_entry(&app, "/usr/bin/chromium", Path::new("/tmp/p"));
        assert!(e.contains("--app=https://e.com/a%%20b"), "got: {e}");
    }

    #[test]
    fn entry_ownership_detection() {
        assert!(entry_is_ours("[Desktop Entry]\nX-TvShell-WebApp=true\n"));
        // Pre-rebrand marker still counts as ours.
        assert!(entry_is_ours("[Desktop Entry]\nX-GameShell-WebApp=true\n"));
        // A foreign entry must never be considered ours.
        assert!(!entry_is_ours(
            "[Desktop Entry]\nName=Firefox\nExec=firefox\n"
        ));
        assert!(!entry_is_ours("[Desktop Entry]\nX-TvShell-WebApp=false\n"));
    }

    #[test]
    fn build_entry_allocates_ids_and_classes() {
        let existing = vec![sample()];
        let a = build_entry("YouTube", "https://youtube.com/tv", &existing).unwrap();
        assert_eq!(a.id, "youtube-2");
        assert_eq!(a.wm_class, "tvshell-youtube-2");
        let b = build_entry("Netflix", "https://netflix.com", &existing).unwrap();
        assert_eq!(b.id, "netflix");
        assert!(build_entry("", "https://x.com", &existing).is_err());
        assert!(build_entry("X", "not-a-url", &existing).is_err());
    }

    #[test]
    fn parse_registry_is_lenient() {
        let good =
            r#"{"webApps":[{"id":"a","name":"A","url":"https://a.io","wmClass":"tvshell-a"}]}"#;
        assert_eq!(parse_registry(good).len(), 1);
        // Missing key, wrong type, and malformed members all degrade to what parses.
        assert!(parse_registry("{}").is_empty());
        assert!(parse_registry(r#"{"webApps":{}}"#).is_empty());
        assert!(parse_registry("not json").is_empty());
        let partial =
            r#"{"webApps":[{"id":"a","name":"A","url":"https://a.io","wmClass":"c"},{"id":"b"}]}"#;
        assert_eq!(parse_registry(partial).len(), 1);
    }

    #[test]
    fn registry_json_roundtrips_field_names() {
        let v = registry_json(&[sample()]);
        assert_eq!(v[0]["wmClass"], "tvshell-youtube");
        assert_eq!(v[0]["id"], "youtube");
        // Must serialize compactly enough to stay on one settings.json line.
        assert!(!serde_json::to_string(&v).unwrap().contains('\n'));
    }

    #[test]
    fn write_refuses_to_clobber_a_foreign_entry() {
        // See `crate::testutil` for why this is based on `current_exe()`
        // rather than the system temp dir.
        let dir = crate::testutil::scratch_dir("webapps-test");

        let app = sample();
        let victim = dir.join(desktop_file_name(&app.id));
        std::fs::write(&victim, "[Desktop Entry]\nName=Important\nExec=/bin/true\n").unwrap();

        let err = write_desktop_entry(&app, "/usr/bin/chromium", &dir).unwrap_err();
        assert!(err.contains("refusing to overwrite"), "got: {err}");
        // The foreign file is untouched.
        assert!(std::fs::read_to_string(&victim)
            .unwrap()
            .contains("Important"));
        // And removal refuses it too.
        assert!(remove_desktop_entry(&app.id, &dir)
            .unwrap_err()
            .contains("refusing to delete"));

        // Our own entry writes and deletes cleanly.
        std::fs::remove_file(&victim).unwrap();
        let written = write_desktop_entry(&app, "/usr/bin/chromium", &dir).unwrap();
        assert!(entry_is_ours(&std::fs::read_to_string(&written).unwrap()));
        remove_desktop_entry(&app.id, &dir).unwrap();
        assert!(!written.exists());
        // Deleting again is a no-op success.
        remove_desktop_entry(&app.id, &dir).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
