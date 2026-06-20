//! Shared action logic used by both the HTTP bridge and the MCP server.
//!
//! All async fns here are cross-platform (no Linux-only imports). The binary
//! (`main.rs`) is Linux-gated, but the logic — intent validation, screenshot
//! capture, log reading, status assembly — compiles and unit-tests on macOS.
//!
//! **Callers** pass in the channel handles they already hold; this module never
//! owns channels directly. Both `http.rs` and `mcp.rs` call into these fns.

use crate::apps;
use crate::protocol::{Event, INTENT_OVERLAY_TARGETS};
use crate::state::Control;
use serde::Serialize;
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, mpsc, oneshot};

// ─── Intent validation ───────────────────────────────────────────────────────

/// Returns `true` when `name` is an intent the shell's QML handler recognises.
///
/// The vocabulary mirrors `INTENT_VOCAB` in `protocol.rs` plus the
/// namespaced deep-link families:
///
/// - `settings:<any>` — open a settings page by slug
/// - `overlay:<target>` — open a QAM overlay (`volume`, `network`, `session`)
/// - `app:<wmClass>` — launch a local app by its StartupWMClass
///
/// Unknown `overlay:` targets are accepted here (the shell degrades gracefully)
/// because the target list may grow without a daemon rebuild.
pub fn is_valid_intent(name: &str) -> bool {
    crate::protocol::is_known_intent(name)
}

/// Build the `intent:<name>` string broadcast on the event bus from a
/// validated `name` token (e.g. `"menu"` → `"intent:menu"`).
///
/// Called by both the IPC server and the HTTP/MCP bridges so the on-wire
/// event payload is always consistent.
pub fn intent_broadcast_name(name: &str) -> String {
    format!("intent:{name}")
}

// ─── Intent dispatch ─────────────────────────────────────────────────────────

/// Dispatch an intent through the control channel and return the daemon's
/// reply string, or `None` when the channel is closed.
///
/// The reply is either `"ok"` or `"error:<message>"`.
pub async fn dispatch_intent(control_tx: &mpsc::Sender<Control>, name: String) -> Option<String> {
    let (reply_tx, reply_rx) = oneshot::channel();
    control_tx
        .send(Control::Intent {
            name,
            reply: reply_tx,
        })
        .await
        .ok()?;
    reply_rx.await.ok()
}

/// Dispatch a key synthesise through the control channel and return the
/// daemon's reply, or `None` when the channel is closed.
///
/// The reply is either `"ok"` or `"error:<message>"`.
pub async fn dispatch_key(control_tx: &mpsc::Sender<Control>, name: String) -> Option<String> {
    let (reply_tx, reply_rx) = oneshot::channel();
    control_tx
        .send(Control::Key {
            name,
            reply: reply_tx,
        })
        .await
        .ok()?;
    reply_rx.await.ok()
}

// ─── Screenshot ──────────────────────────────────────────────────────────────

/// Capture the Wayland display to raw PNG bytes via `grim -`.
///
/// When `flash` is `true`, [`Event::ScreenshotFlash`] is broadcast on
/// `events_tx` immediately after a successful capture — before returning —
/// so the QML shell can paint a brief white vignette. The overlay never
/// appears in the captured PNG because the flash fires **after** `grim`
/// completes.
///
/// Returns `Ok(png_bytes)` on success, `Err(message)` on any failure.
pub async fn capture_screenshot(
    events_tx: &broadcast::Sender<Event>,
    flash: bool,
) -> Result<Vec<u8>, String> {
    let mut cmd = tokio::process::Command::new("grim");
    cmd.arg("-");
    // Inject all available session env vars (WAYLAND_DISPLAY,
    // HYPRLAND_INSTANCE_SIGNATURE, XDG_RUNTIME_DIR) so grim finds the live
    // Wayland socket even when the daemon was started before `exec Hyprland`.
    for (k, v) in crate::session_env::session_env_pairs() {
        cmd.env(k, v);
    }
    match cmd.output().await {
        Ok(out) if out.status.success() => {
            if flash {
                // Ignored when there are no broadcast subscribers.
                let _ = events_tx.send(Event::ScreenshotFlash);
            }
            Ok(out.stdout)
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(format!("grim failed: {}", stderr.trim()))
        }
        Err(e) => Err(format!("grim error: {e}")),
    }
}

/// Capture-time provenance returned alongside a screenshot so a caller can tell
/// *which* deployed game-shell produced the frame — distinguishing latest `main`,
/// a feature branch, or another agent's checkout at a glance.
///
/// **Read live per capture, never cached:** a `dev_deploy` mutates HEAD under the
/// long-lived daemon (it does `git checkout` + `reset --hard` without restarting
/// the process), so the deployed SHA/branch can change between two screenshots.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CaptureMeta {
    /// RFC3339 UTC timestamp of when the metadata was gathered — immediately
    /// after the PNG was captured. `"unknown"` if the clock format fails.
    pub captured_at: String,
    /// Short git SHA of the checked-out repo at `install_root()`.
    pub sha: String,
    /// Current branch name, or `"HEAD"` on a detached checkout (tag/SHA).
    pub branch: String,
    /// Version string from `Cargo.toml`.
    pub version: &'static str,
}

/// Resolve the short SHA and branch name of the repo at `root`.
///
/// Two `git rev-parse` invocations are run concurrently: `--short HEAD` and
/// `--abbrev-ref HEAD`. They can't be combined into one call because both flags
/// are global and would apply to every listed revision. A detached HEAD yields
/// `branch == "HEAD"`. Any failure degrades to `"unknown"`.
async fn git_sha_and_branch(root: &std::path::Path) -> (String, String) {
    let Some(root_str) = root.to_str() else {
        return ("unknown".to_owned(), "unknown".to_owned());
    };
    let run = |args: [&'static str; 1]| {
        let root_str = root_str.to_owned();
        let extra = args[0];
        async move {
            let out = tokio::process::Command::new("git")
                .args(["-C", &root_str, "rev-parse", extra, "HEAD"])
                .output()
                .await;
            match out {
                Ok(o) if o.status.success() => {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_owned();
                    if s.is_empty() {
                        "unknown".to_owned()
                    } else {
                        s
                    }
                }
                _ => "unknown".to_owned(),
            }
        }
    };
    tokio::join!(run(["--short"]), run(["--abbrev-ref"]))
}

/// Gather [`CaptureMeta`] for the current capture. See the struct docs for why
/// this is read live rather than cached.
pub async fn capture_meta() -> CaptureMeta {
    let root = crate::session_env::install_root();
    let (sha, branch) = git_sha_and_branch(&root).await;
    let captured_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_owned());
    CaptureMeta {
        captured_at,
        sha,
        branch,
        version: env!("CARGO_PKG_VERSION"),
    }
}

// ─── Status ──────────────────────────────────────────────────────────────────

/// Serialisable status blob returned by `get_status` / `GET /dev/status`.
///
/// The `JsonSchema` derive is conditional on the `mcp` feature (which pulls in
/// `schemars` transitively via `rmcp`). When MCP is disabled the struct is still
/// fully usable — it just lacks the output-schema machinery.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct StatusInfo {
    /// Short git SHA of the checked-out repo at `install_root()`.
    pub sha: String,
    /// PID of the running daemon process.
    pub daemon_pid: u32,
    /// Version string from `Cargo.toml`.
    pub version: &'static str,
    /// Whether `pgrep -x quickshell` exits zero.
    pub shell_running: bool,
    /// `WAYLAND_DISPLAY` if resolvable, else `null`.
    pub wayland_display: Option<String>,
    /// `true` when `HYPRLAND_INSTANCE_SIGNATURE` is resolvable.
    pub hypr_sig_present: bool,
}

/// Assemble the [`StatusInfo`] by querying the environment and running git.
pub async fn get_status() -> StatusInfo {
    let root = crate::session_env::install_root();

    let (sha, _branch) = git_sha_and_branch(&root).await;

    let shell_running = matches!(
        tokio::process::Command::new("pgrep")
            .args(["-x", "quickshell"])
            .output()
            .await,
        Ok(o) if o.status.success()
    );

    StatusInfo {
        sha,
        daemon_pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION"),
        shell_running,
        wayland_display: crate::session_env::resolve_wayland_display(),
        hypr_sig_present: crate::session_env::resolve_hypr_signature().is_some(),
    }
}

// ─── Logs ────────────────────────────────────────────────────────────────────

const QS_LOG_PATH: &str = "/tmp/qs-log.txt";

/// Read the quickshell log file, apply optional `filter`, and return the
/// last `lines` lines.
///
/// Returns an explanatory string (not an error) when the log file does not
/// exist yet — callers may display it as-is. `Err` is returned only on
/// unexpected I/O failures.
pub fn get_logs(lines: usize, filter: Option<&str>) -> Result<String, String> {
    let content = match std::fs::read_to_string(QS_LOG_PATH) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(
                "(no /tmp/qs-log.txt yet — POST /dev/restart-shell to capture quickshell logs)\n"
                    .to_owned(),
            );
        }
        Err(e) => return Err(format!("read log failed: {e}")),
    };

    let filtered: Vec<&str> = match filter {
        Some(f) => {
            let f_lower = f.to_lowercase();
            content
                .lines()
                .filter(|l| l.to_lowercase().contains(&f_lower))
                .collect()
        }
        None => content.lines().collect(),
    };

    let tail: String = filtered
        .iter()
        .rev()
        .take(lines)
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!("{tail}\n"))
}

// ─── Dev operations ──────────────────────────────────────────────────────────

/// Run a git command in `root`, returning `(success, combined_stdout_stderr)`.
async fn git_run(root: &std::path::Path, args: &[&str]) -> (bool, String) {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await;
    match out {
        Ok(o) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            (o.status.success(), combined)
        }
        Err(e) => (false, format!("git error: {e}")),
    }
}

/// `POST /dev/deploy[?ref=<ref>]` — git fetch + checkout + reset to remote.
///
/// Returns a short status line on success, or `Err(message)` on failure.
pub async fn dev_deploy(git_ref: Option<&str>) -> Result<String, String> {
    let root = crate::session_env::install_root();
    let r = git_ref.unwrap_or("main");

    // Validate the ref before any git call (Fix 4).
    validate_git_ref(r)?;

    tracing::info!("dev/deploy: ref={r} root={}", root.display());

    let root_str = root
        .to_str()
        .ok_or("install root path is not valid UTF-8")?;

    // Fetch
    let (ok, out) = git_run(&root, &["-C", root_str, "fetch", "origin", "--prune"]).await;
    if !ok {
        return Err(format!("git fetch failed: {out}"));
    }

    // Checkout. The trailing `--` forces git to treat `r` as a revision, never a
    // pathspec, so a ref that happens to collide with a tracked path name can't be
    // misinterpreted (validate_git_ref already blocks option-like and exotic refs).
    let (ok, out) = git_run(&root, &["-C", root_str, "checkout", "-f", r, "--"]).await;
    if !ok {
        return Err(format!("git checkout failed: {out}"));
    }

    // Reset to remote tracking branch (if it exists)
    let remote_ref = format!("origin/{r}");
    let (has_remote, _) = git_run(
        &root,
        &[
            "-C",
            root_str,
            "rev-parse",
            "--verify",
            "--quiet",
            &remote_ref,
        ],
    )
    .await;
    if has_remote {
        let (ok, out) = git_run(&root, &["-C", root_str, "reset", "--hard", &remote_ref]).await;
        if !ok {
            return Err(format!("git reset failed: {out}"));
        }
    }

    // Report resulting SHA
    let (ok, sha_out) = git_run(&root, &["-C", root_str, "rev-parse", "--short", "HEAD"]).await;
    if !ok {
        return Err(format!("git rev-parse failed: {sha_out}"));
    }
    let sha = sha_out.trim();
    Ok(format!("deployed {r} @ {sha}\n"))
}

/// `POST /dev/build` — build via scripts/build-daemon.sh + install the binary.
///
/// Returns the last 12 lines of cargo stderr on success, `Err(message)` on failure.
pub async fn dev_build() -> Result<String, String> {
    let root = crate::session_env::install_root();
    let daemon_dir = root.join("daemon");
    tracing::info!("dev/build: cwd={}", daemon_dir.display());

    let build_script = root.join("scripts/build-daemon.sh");
    let out = tokio::process::Command::new("bash")
        .arg(&build_script)
        .env("GAME_SHELL_ROOT", &root)
        .current_dir(&root)
        .output()
        .await;

    match out {
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let tail: String = stderr
                .lines()
                .rev()
                .take(12)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");

            if !o.status.success() {
                return Err(format!("cargo build failed:\n{tail}"));
            }

            // Install the binary. Since the repo is a Cargo workspace, cargo
            // writes to the workspace-root `target/` (shared target dir) even
            // when invoked from `daemon/` — so the build output is at
            // `<root>/target/release/...`, NOT `daemon/target/...`.
            let src = root.join("target/release/game-shell-input");
            let dst = root.join("bin/game-shell-input");
            let install = tokio::process::Command::new("install")
                .args(["-m755"])
                .arg(&src)
                .arg(&dst)
                .output()
                .await;

            match install {
                Ok(i) if i.status.success() => Ok(format!("{tail}\nok\n")),
                Ok(i) => {
                    let err = String::from_utf8_lossy(&i.stderr);
                    Err(format!("install failed: {err}"))
                }
                Err(e) => Err(format!("install error: {e}")),
            }
        }
        Err(e) => Err(format!("cargo error: {e}")),
    }
}

/// `POST /dev/restart-shell` — kill quickshell and relaunch detached.
///
/// Returns a brief startup summary (no errors seen / first WARN/ERROR lines).
pub async fn dev_restart_shell() -> Result<String, String> {
    // Kill existing quickshell (ignore failure — it may not be running).
    let _ = tokio::process::Command::new("pkill")
        .args(["-x", "quickshell"])
        .output()
        .await;

    // Open the log file and spawn quickshell detached on the blocking pool: the
    // log open + the `setsid` fork are blocking syscalls, and the detached spawn
    // must stay on `std::process::Command` (tokio::process would try to reap the
    // child we deliberately let outlive this handler). Done together so the new
    // session is created in one off-runtime hop.
    let env_pairs = crate::session_env::session_env_pairs();
    let spawned = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(QS_LOG_PATH)
            .map_err(|e| format!("open log failed: {e}"))?;
        let log_stderr = log_file
            .try_clone()
            .map_err(|e| format!("clone log fd failed: {e}"))?;

        // Spawn quickshell detached (new session so it outlives this handler task).
        let mut cmd = std::process::Command::new("setsid");
        cmd.args(["quickshell", "-c", "game-shell"]);
        cmd.stdout(log_file);
        cmd.stderr(log_stderr);
        for (k, v) in env_pairs {
            cmd.env(k, v);
        }
        cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
        Ok(())
    })
    .await;
    match spawned {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(format!("restart task failed: {e}")),
    }

    // Give quickshell a moment to emit initial log lines.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let log_content = tokio::fs::read_to_string(QS_LOG_PATH)
        .await
        .unwrap_or_default();
    let filtered: String = log_content
        .lines()
        .filter(|l| {
            let upper = l.to_uppercase();
            (upper.contains("WARN") || upper.contains("ERROR"))
                && !upper.contains("COULD NOT LOAD ICON")
        })
        .rev()
        .take(30)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");

    if filtered.is_empty() {
        Ok("started (no WARN/ERROR in first 3s)\n".to_owned())
    } else {
        Ok(format!("{filtered}\n"))
    }
}

/// Request daemon re-exec.
///
/// Sets the reexec flag (read after the runtime shuts down to decide whether to
/// `exec()`) and cancels the shared shutdown token (which stops the MCP server
/// and unblocks the main `select!` in a race-free way — `CancellationToken` is
/// multi-consumer safe, unlike `Notify::notify_one()`).
pub fn request_reexec(
    reexec_flag: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    shutdown: &tokio_util::sync::CancellationToken,
) {
    reexec_flag.store(true, std::sync::atomic::Ordering::Release);
    shutdown.cancel();
}

// ─── Auth helpers ─────────────────────────────────────────────────────────────

/// Constant-time string comparison to prevent timing oracles.
///
/// Uses `subtle::ConstantTimeEq` on the UTF-8 byte representations. A length
/// mismatch leaks only the length (not which character differs), which is
/// acceptable for a fixed-format `"Bearer <token>"` prefix.
///
/// Shared by both the HTTP bridge (`http.rs`) and the MCP auth middleware
/// (`mcp.rs`) so both bridges use identical comparison logic.
pub fn ct_eq_str(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

// ─── Git ref validation ───────────────────────────────────────────────────────

/// Validate a git ref before passing it to a `git` subprocess.
///
/// Accepts refs that match `^[A-Za-z0-9._/-]+$` and rejects:
/// - refs starting with `-` (flag injection: `-f`, `--exec`, etc.)
/// - refs containing characters outside the safe set
///
/// Returns `Ok(())` when the ref is safe, `Err(message)` otherwise.
pub fn validate_git_ref(r: &str) -> Result<(), String> {
    if r.is_empty() {
        return Err("git ref must not be empty".to_owned());
    }
    if r.starts_with('-') {
        return Err(format!("invalid git ref '{r}': must not start with '-'"));
    }
    if !r
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
    {
        return Err(format!(
            "invalid git ref '{r}': only A-Z a-z 0-9 . _ / - are allowed"
        ));
    }
    Ok(())
}

// ─── Convenience sugar tools ─────────────────────────────────────────────────

/// Build an `intent:settings:<page>` name. The page slug is passed through
/// unvalidated — unknown slugs are a graceful no-op in QML.
pub fn settings_intent(page: &str) -> String {
    format!("settings:{page}")
}

/// Build an `intent:overlay:<target>` name.
/// Returns `Err` when `target` is not in the known overlay vocabulary.
pub fn overlay_intent(target: &str) -> Result<String, String> {
    if INTENT_OVERLAY_TARGETS.contains(&target) {
        Ok(format!("overlay:{target}"))
    } else {
        let valid = INTENT_OVERLAY_TARGETS.join(", ");
        Err(format!("unknown overlay target '{target}'; valid: {valid}"))
    }
}

/// Build an `intent:app:<wm_class>` name.
pub fn app_intent(wm_class: &str) -> String {
    format!("app:{wm_class}")
}

// ─── UI state ────────────────────────────────────────────────────────────────

/// Lightweight UI-observable state returned by `get_ui_state`.
///
/// Reports the Hyprland-level active window (which application currently holds
/// focus) and whether quickshell is the focused process. This is **window-level
/// state from the compositor**, not QML-internal state (which settings page is
/// open, what list item is selected) — QML does not report that to the daemon.
/// Use `take_screenshot` to observe QML-internal state.
///
/// On non-Linux builds (macOS dev) the Hyprland fields are always absent and
/// `platform_note` explains why.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct UiState {
    /// Whether quickshell is currently the active (focused) Hyprland window.
    /// `None` when the active-window query is unavailable (non-Linux or no
    /// Hyprland socket).
    pub quickshell_focused: Option<bool>,
    /// Class of the currently focused window (from Hyprland's `j/activewindow`).
    /// `None` when nothing is focused or the query fails.
    pub active_window_class: Option<String>,
    /// Title of the currently focused window. `None` when nothing is focused.
    pub active_window_title: Option<String>,
    /// Whether quickshell is running at all (via `pgrep -x quickshell`).
    pub shell_running: bool,
    /// Present only on non-Linux builds; explains which fields are unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_note: Option<&'static str>,
}

/// Query lightweight UI / compositor state without a screenshot.
///
/// On Linux, performs a one-shot Hyprland `j/activewindow` query to determine
/// which window is focused and whether it is quickshell. On non-Linux builds the
/// Hyprland fields are absent (the function still compiles and returns a degraded
/// result with a `platform_note`).
pub async fn get_ui_state() -> UiState {
    let shell_running = matches!(
        tokio::process::Command::new("pgrep")
            .args(["-x", "quickshell"])
            .output()
            .await,
        Ok(o) if o.status.success()
    );

    #[cfg(target_os = "linux")]
    {
        use crate::hyprland;
        let active_json = hyprland::query_active_window().await;
        // Parse the {class, title} from the JSON; empty object = nothing focused.
        let parsed: serde_json::Value =
            serde_json::from_str(&active_json).unwrap_or(serde_json::Value::Null);
        let class = parsed
            .get("class")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_owned());
        let title = parsed
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_owned());
        let quickshell_focused = class.as_deref().map(|c| {
            // quickshell registers as class "quickshell" in Hyprland.
            c.eq_ignore_ascii_case("quickshell")
        });
        UiState {
            quickshell_focused,
            active_window_class: class,
            active_window_title: title,
            shell_running,
            platform_note: None,
        }
    }

    #[cfg(not(target_os = "linux"))]
    UiState {
        quickshell_focused: None,
        active_window_class: None,
        active_window_title: None,
        shell_running,
        platform_note: Some(
            "Hyprland window state is only available on Linux; \
             shell_running is available on all platforms.",
        ),
    }
}

// ─── App discovery ───────────────────────────────────────────────────────────

/// A single installed application, as returned by the `list_apps` MCP tool.
///
/// A trimmed view of [`apps::App`] exposing only the fields an agent needs:
/// `name` (human-readable display name), `wm_class` (pass to `launch_app`),
/// and `comment` (optional short description). `exec` and `icon` are omitted —
/// they are implementation details the agent does not need.
///
/// The `JsonSchema` derive is conditional on the `mcp` feature (same gate as
/// [`StatusInfo`]).
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AppEntry {
    /// Human-readable application name (from the .desktop `Name` field).
    pub name: String,
    /// StartupWMClass from the .desktop file — pass this to `launch_app`.
    pub wm_class: String,
    /// Optional short description from the .desktop `Comment` field.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

/// Object wrapper for the `list_apps` tool result.
///
/// MCP requires a structured tool's `outputSchema` to have an `object` root
/// type. A bare `Vec<AppEntry>` serialises to a JSON array root, which rmcp
/// rejects at tool-router build time (a runtime panic, hit on every request).
/// Nesting the list under `apps` keeps the root an object.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ListAppsResult {
    /// Installed applications discovered from `.desktop` files.
    pub apps: Vec<AppEntry>,
}

impl From<apps::App> for AppEntry {
    fn from(a: apps::App) -> Self {
        AppEntry {
            name: a.name,
            wm_class: a.wm_class,
            comment: a.comment,
        }
    }
}

/// Scan the standard XDG application directories and return the app list as
/// [`AppEntry`] values.
///
/// Runs `apps::scan_apps` on a blocking thread so the async reactor is not
/// stalled by directory I/O.  Cross-platform: `apps.rs` uses only the standard
/// library and the pure `freedesktop-desktop-entry` crate — no Linux-only APIs.
pub async fn list_apps() -> Vec<AppEntry> {
    tokio::task::spawn_blocking(|| apps::scan_apps().into_iter().map(AppEntry::from).collect())
        .await
        .unwrap_or_default()
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::INTENT_VOCAB;

    // ── intent validation ────────────────────────────────────────────────────

    #[test]
    fn known_vocab_intents_are_valid() {
        for &name in INTENT_VOCAB {
            assert!(
                is_valid_intent(name),
                "expected '{name}' to be valid intent"
            );
        }
    }

    #[test]
    fn settings_deep_link_is_valid() {
        assert!(is_valid_intent("settings:bluetooth"));
        assert!(is_valid_intent("settings:audio"));
        assert!(is_valid_intent("settings:anything-at-all"));
    }

    #[test]
    fn overlay_known_targets_are_valid() {
        assert!(is_valid_intent("overlay:volume"));
        assert!(is_valid_intent("overlay:network"));
        assert!(is_valid_intent("overlay:session"));
    }

    #[test]
    fn overlay_unknown_target_is_invalid() {
        assert!(!is_valid_intent("overlay:doesnotexist"));
    }

    #[test]
    fn app_deep_link_is_valid() {
        assert!(is_valid_intent("app:steam"));
        assert!(is_valid_intent("app:org.mozilla.Firefox"));
    }

    #[test]
    fn unknown_intent_is_invalid() {
        assert!(!is_valid_intent(""));
        assert!(!is_valid_intent("unknown-thing"));
        assert!(!is_valid_intent("foo:bar"));
    }

    // ── status struct serialises to expected JSON fields ─────────────────────

    #[test]
    fn status_info_serialises() {
        let s = StatusInfo {
            sha: "abc1234".into(),
            daemon_pid: 42,
            version: "0.1.0",
            shell_running: true,
            wayland_display: Some("wayland-1".into()),
            hypr_sig_present: false,
        };
        let json = serde_json::to_string(&s).expect("serialise StatusInfo");
        assert!(json.contains("\"sha\":\"abc1234\""));
        assert!(json.contains("\"daemon_pid\":42"));
        assert!(json.contains("\"version\":\"0.1.0\""));
        assert!(json.contains("\"shell_running\":true"));
        assert!(json.contains("\"wayland_display\":\"wayland-1\""));
        assert!(json.contains("\"hypr_sig_present\":false"));
    }

    #[test]
    fn capture_meta_serialises() {
        let m = CaptureMeta {
            captured_at: "2026-06-14T18:26:00Z".into(),
            sha: "a1b2c3d".into(),
            branch: "feat/screenshot-metadata".into(),
            version: "0.1.0",
        };
        let json = serde_json::to_string(&m).expect("serialise CaptureMeta");
        assert!(json.contains("\"captured_at\":\"2026-06-14T18:26:00Z\""));
        assert!(json.contains("\"sha\":\"a1b2c3d\""));
        assert!(json.contains("\"branch\":\"feat/screenshot-metadata\""));
        assert!(json.contains("\"version\":\"0.1.0\""));
    }

    #[test]
    fn status_info_null_wayland() {
        let s = StatusInfo {
            sha: "abc".into(),
            daemon_pid: 1,
            version: "0.1.0",
            shell_running: false,
            wayland_display: None,
            hypr_sig_present: false,
        };
        let json = serde_json::to_string(&s).expect("serialise StatusInfo");
        assert!(json.contains("\"wayland_display\":null"));
    }

    // ── logs filter ──────────────────────────────────────────────────────────

    #[test]
    fn get_logs_no_file_returns_ok_placeholder() {
        // A path that definitely does not exist — expect Ok with a hint.
        // We can't override the path constant, but we can test the public fn
        // indirectly by confirming it returns Ok (not Err) on NotFound.
        // This test only verifies the NotFound arm; the real fs test needs a
        // temp file, which is an integration test concern.
        let result = get_logs(10, None);
        // Either the log exists (Ok with content) or it doesn't (Ok with placeholder).
        // Either way it should NOT be an Err on a normal dev box.
        // On CI, /tmp/qs-log.txt is unlikely to exist — we check both cases.
        match result {
            Ok(s) => {
                // Either real content or the placeholder.
                assert!(!s.is_empty(), "get_logs should return non-empty string");
            }
            Err(e) => {
                // Only acceptable if not a NotFound error (some other I/O failure
                // is theoretically possible in a restricted environment).
                panic!("get_logs returned Err unexpectedly: {e}");
            }
        }
    }

    #[test]
    fn overlay_intent_known_target() {
        assert_eq!(overlay_intent("volume"), Ok("overlay:volume".to_owned()));
        assert_eq!(overlay_intent("network"), Ok("overlay:network".to_owned()));
    }

    #[test]
    fn overlay_intent_unknown_target_err() {
        let result = overlay_intent("oops");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("oops"));
        assert!(msg.contains("valid:"));
    }

    #[test]
    fn settings_intent_passthrough() {
        assert_eq!(settings_intent("bluetooth"), "settings:bluetooth");
        assert_eq!(settings_intent("anything"), "settings:anything");
    }

    #[test]
    fn app_intent_passthrough() {
        assert_eq!(app_intent("steam"), "app:steam");
    }

    // ── git ref validation (Fix 4) ───────────────────────────────────────────

    #[test]
    fn git_ref_accepts_valid_refs() {
        // branch names
        assert!(validate_git_ref("main").is_ok());
        assert!(validate_git_ref("feat/mcp-bridge").is_ok());
        assert!(validate_git_ref("release-1.2.3").is_ok());
        assert!(validate_git_ref("v0.1.0").is_ok());
        // short SHA
        assert!(validate_git_ref("a1b2c3d").is_ok());
        // full SHA-like string
        assert!(validate_git_ref("abc123def456").is_ok());
        // nested path (rare but valid git)
        assert!(validate_git_ref("refs/heads/main").is_ok());
    }

    #[test]
    fn git_ref_rejects_leading_dash() {
        assert!(validate_git_ref("-f").is_err());
        assert!(validate_git_ref("--exec").is_err());
        assert!(validate_git_ref("-").is_err());
    }

    #[test]
    fn git_ref_rejects_empty() {
        assert!(validate_git_ref("").is_err());
    }

    #[test]
    fn git_ref_rejects_special_chars() {
        assert!(validate_git_ref("main;rm -rf /").is_err());
        assert!(validate_git_ref("main|evil").is_err());
        assert!(validate_git_ref("$(evil)").is_err());
        assert!(validate_git_ref("main\nevil").is_err());
        assert!(validate_git_ref("main\tevil").is_err());
        // Spaces are not in the allowed set
        assert!(validate_git_ref("main evil").is_err());
    }

    // ── ct_eq_str ────────────────────────────────────────────────────────────

    #[test]
    fn ct_eq_str_equal() {
        assert!(ct_eq_str("Bearer secret123", "Bearer secret123"));
    }

    #[test]
    fn ct_eq_str_different() {
        assert!(!ct_eq_str("Bearer secret123", "Bearer secret124"));
    }

    #[test]
    fn ct_eq_str_different_lengths() {
        assert!(!ct_eq_str("short", "much longer string"));
        assert!(!ct_eq_str("", "nonempty"));
    }

    #[test]
    fn ct_eq_str_empty_equal() {
        assert!(ct_eq_str("", ""));
    }

    // ── StatusInfo JsonSchema derive (mcp feature) ───────────────────────────
    //
    // This test is compiled only when the `mcp` feature is active — that's the
    // same gate as the `schemars::JsonSchema` derive on `StatusInfo`. It verifies:
    //   1. The derive compiles and produces a schema (no runtime panic from an
    //      invalid schema type, e.g. one that isn't "object").
    //   2. The schema contains the expected field names so a client that depends
    //      on the output schema won't silently get an empty/wrong object.
    //   3. The struct still serialises the same way (no regression from adding
    //      the derive — `JsonSchema` is additive).
    #[cfg(feature = "mcp")]
    #[test]
    fn status_info_json_schema_derive_works() {
        use schemars::schema_for;

        let schema = schema_for!(StatusInfo);
        let schema_json = serde_json::to_string(&schema).expect("schema serialises");

        // Schema must contain every public field name.
        for field in &[
            "sha",
            "daemon_pid",
            "version",
            "shell_running",
            "wayland_display",
            "hypr_sig_present",
        ] {
            assert!(
                schema_json.contains(field),
                "expected field '{field}' in StatusInfo schema: {schema_json}"
            );
        }
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn status_info_schema_is_object_type() {
        use schemars::schema_for;
        let schema = schema_for!(StatusInfo);
        // schema_for! always wraps in a RootSchema; the inner schema's instance_type
        // must be object (required by the MCP spec for output schemas).
        let schema_json = serde_json::to_value(&schema).expect("schema serialises to value");
        // The generated schema has a "type": "object" field at the root properties
        // level (schemars 1.x wraps in `{"$schema":..., "title":..., ...}`).
        // Check that the word "object" appears somewhere in the serialised schema.
        let schema_str = schema_json.to_string();
        assert!(
            schema_str.contains("\"object\""),
            "StatusInfo schema should declare type:object, got: {schema_str}"
        );
    }
}
