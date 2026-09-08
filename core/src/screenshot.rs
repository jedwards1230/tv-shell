//! Screen capture: the `screenshot <path>` verb.
//!
//! # Why this exists at all
//!
//! v1's entire screenshot path is `grim`, and `grim` cannot work under
//! gamescope. gamescope 3.16.28 implements **no Wayland screen-capture
//! protocol** — neither `wlr-screencopy-unstable-v1` nor
//! `ext-image-copy-capture-v1` appears anywhere in its `protocol/` directory or
//! its sources — so `grim` fails with "compositor doesn't support the screen
//! capture protocol" by construction, not by misconfiguration (measured against
//! the live v2 session, 2026-09-08). Every v1 surface built on `grim` — the
//! daemon's `GET /screenshot`, the MCP `take_screenshot` tool, the panel's
//! screenshot page, `docs/qa-screenshot-views.md` — is therefore unavailable
//! under v2, and the agent-native dev loop's observe → act → **verify** cycle
//! has no verify step until something replaces it.
//!
//! The X11 fallback does not work either, and that is worth writing down so
//! nobody spends a second afternoon on it. gamescope runs its Xwayland servers
//! `-rootless` and calls `XCompositeRedirectSubwindows(..., CompositeRedirectManual)`,
//! so `XGetImage` on the Xwayland root fails `BadMatch`, and readback of a
//! GPU-rendered client window (via the window or via
//! `XCompositeNameWindowPixmap`) returns 100% black — a DRI3 client's frames
//! live in a buffer the X server passes through and never rasterizes. Both
//! measured on the box. And the v2 shell is not an X client in the first place:
//! the session runs `--expose-wayland`, so the shell is an xdg-shell Wayland
//! client with no X window to capture.
//!
//! # The mechanism: ask the compositor, do not read the screen
//!
//! What 3.16.28 *does* have is a root-window request property. Setting
//! `GAMESCOPECTRL_REQUEST_SCREENSHOT` (`src/steamcompmgr.cpp:8133`, handled at
//! `:6258`) makes gamescope composite on its next repaint and write the frame to
//! disk itself. So the core never reads pixels: it expresses an intent and
//! harvests the result, the same shape as every other verb here.
//!
//! Three properties of that path drive this module's design:
//!
//! 1. **The output path is hardcoded** to [`GAMESCOPE_OUTPUT`]
//!    (`steamcompmgr.cpp:6264`). Not an env var, not a flag. Every X-requested
//!    capture on the box lands on that one path and overwrites the last.
//! 2. **The property's value is a screenshot TYPE, not flags** — the
//!    `gamescope_control_screenshot_type` enum from
//!    `protocol/gamescope-control.xml:83-88`. See [`FULL_COMPOSITION`].
//! 3. **gamescope deletes the property when the attempt finishes**, from the
//!    encoder thread, after the file is closed (`:3295`). That deletion is the
//!    only completion signal the X path has — and it fires on the write-FAILURE
//!    path too (`:3316`).
//!
//! # The two rules, and why they are rules
//!
//! **Rule 1 — the property clearing means the attempt FINISHED, never that it
//! SUCCEEDED.** Point 3: gamescope clears on failure as well. An implementation
//! that replied `ok` the moment the property vanished would report success for a
//! capture that produced nothing — precisely the class [`crate::baselayer`]
//! exists to eliminate.
//!
//! **Rule 2 — a stale screenshot is made unrepresentable, not merely
//! detected.** Point 1 is the trap. Because the path is fixed and shared, a
//! capture that never happened leaves the PREVIOUS capture sitting there: a
//! valid PNG, of the right size, showing the wrong moment. Nothing about the
//! file distinguishes it, and an agent driving a UI it cannot otherwise see
//! would take the last screen for the current one and "verify" a change that
//! never rendered. So this module **removes the output path before it asks**
//! ([`CaptureFile::clear`]). Anything present afterwards was written by this
//! request, and staleness stops being something that can be got wrong.
//!
//! A `clear` that fails is therefore an error and not a warning: it is exactly
//! the state in which every check after it stops proving anything.
//!
//! # What is deliberately NOT here
//!
//! * **No screenshot type on the wire.** The verb captures
//!   [`FULL_COMPOSITION`] and nothing else. The other types are either wrong for
//!   this job — `base_plane_only` renders the game plane alone, at the *nested*
//!   resolution, missing every overlay the shell draws — or measurably wrong on
//!   this display: `screen_buffer` skips the inverse-EOTF step (`:3437`) and
//!   dumps the raw PQ scanout buffer into an 8-bit sRGB PNG, measured on the
//!   deploy box's HDR session as visibly washed out with lifted blacks. A knob
//!   whose other positions are all defects is not a knob.
//! * **No HDR/AVIF capture.** True 10-bit capture needs a `.avif` destination,
//!   and the X path cannot request one: the extension is baked into the
//!   hardcoded path. `full_composition` to PNG is tone-mapped to gamma 2.2 by
//!   gamescope itself (`:3496-3505`), which is what a QA screenshot wants.
//! * **No configurable source path.** [`GAMESCOPE_OUTPUT`] is a constant because
//!   it is a constant upstream. A config key whose only correct value is the one
//!   already written here is a way to break the capture, not to tune it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::atoms::{AtomConn, AtomError};

/// `gamescope_control_screenshot_type::full_composition` — "every layer w/ no
/// display color mgmt and no mura" (`protocol/gamescope-control.xml:86`).
///
/// The only type this core requests. It captures at the OUTPUT resolution and
/// includes every layer, which is what makes it right for a shell screenshot:
/// the shell's own surface, any overlay above it, and the app underneath all
/// land in one frame. See the module docs for why the other three are not
/// offered.
pub const FULL_COMPOSITION: u32 = 3;

/// Where gamescope writes an X-requested screenshot.
///
/// Hardcoded upstream at `src/steamcompmgr.cpp:6264` as a string literal. The
/// Wayland (`gamescopectl screenshot <path>`) and `SIGUSR2` paths pick other
/// names; the X property path always writes here.
pub const GAMESCOPE_OUTPUT: &str = "/tmp/gamescope.png";

/// How long the compositor gets to complete a capture.
///
/// A guess with headroom, not a measurement. gamescope stores the request and
/// raises `hasRepaint`, but — unlike `force_repaint` — it does **not** call
/// `nudge_steamcompmgr()`, so the capture waits for the next vblank-gated
/// repaint and then for a detached thread to encode and write the PNG. One live
/// capture on the deploy box took roughly a second at 4K. Five seconds is
/// enough that a
/// loaded box cannot produce a false failure, and short enough that a wedged
/// compositor says so inside one interaction. [`Captured::took_ms`] is what
/// would replace this guess with a number.
///
/// The SINGLE source of the value: [`crate::config::SessionConfig`]'s default
/// derives `screenshot_timeout_ms` from it rather than repeating the literal.
pub const DEFAULT_SCREENSHOT_TIMEOUT: Duration = Duration::from_millis(5_000);

/// How often the wait loop re-reads the request property.
///
/// Polled rather than event-driven for the same reason [`crate::baselayer`]
/// polls: a poll cannot silently process nothing.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The compositor half of a capture, as a seam.
///
/// Exists so [`capture`] — the function that decides whether a screenshot
/// happened — is testable **with no X server and no gamescope**. Both rules
/// above live in `capture`, and a rule whose test needs hardware is a rule with
/// no test.
pub trait ScreenshotSurface {
    /// Ask for a capture of `kind`. One property write.
    fn request(&self, kind: u32) -> Result<(), AtomError>;
    /// Is the request property still set? `true` means the compositor has not
    /// finished the attempt. Per Rule 1 it says nothing about whether it worked.
    fn outstanding(&self) -> Result<bool, AtomError>;
}

impl ScreenshotSurface for AtomConn {
    fn request(&self, kind: u32) -> Result<(), AtomError> {
        AtomConn::request_screenshot(self, kind)
    }
    fn outstanding(&self) -> Result<bool, AtomError> {
        AtomConn::screenshot_requested(self)
    }
}

/// The filesystem half of a capture, as a seam.
///
/// Separate from [`ScreenshotSurface`] because the two fail independently, and
/// the interesting cases are exactly the combinations: a compositor that
/// finishes having written nothing, and a compositor that finishes over a file
/// that was already there.
pub trait CaptureFile {
    /// Remove the output path. **Absent is success** — there is nothing to clear
    /// on the first capture after a boot.
    ///
    /// This is Rule 2. A failure here fails the whole capture rather than
    /// warning, because after it nothing downstream can distinguish this
    /// request's frame from the previous one's.
    fn clear(&self) -> std::io::Result<()>;
    /// Does the output file exist now?
    fn present(&self) -> bool;
    /// Move the output file to `dest`, returning its size in bytes.
    fn take(&self, dest: &Path) -> std::io::Result<u64>;
}

/// [`CaptureFile`] over the real [`GAMESCOPE_OUTPUT`] path.
#[derive(Debug, Clone)]
pub struct GamescopeOutput {
    source: PathBuf,
}

impl Default for GamescopeOutput {
    fn default() -> Self {
        Self {
            source: PathBuf::from(GAMESCOPE_OUTPUT),
        }
    }
}

impl CaptureFile for GamescopeOutput {
    fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.source) {
            Ok(()) => Ok(()),
            // The ordinary first-capture-after-boot case. Not an error, and not
            // worth a log line.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn present(&self) -> bool {
        self.source.is_file()
    }

    fn take(&self, dest: &Path) -> std::io::Result<u64> {
        // `rename` first: atomic, and it takes the frame OUT of the shared path,
        // so a later failed capture cannot find this one lying around even if
        // `clear` were ever weakened. It only works within one filesystem — the
        // source is on `/tmp` (tmpfs) and a destination elsewhere is not — so
        // `EXDEV` falls back to copy-then-remove. Every other error is real.
        match std::fs::rename(&self.source, dest) {
            Ok(()) => std::fs::metadata(dest).map(|m| m.len()),
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                let bytes = std::fs::copy(&self.source, dest)?;
                // A failed unlink after a good copy leaves the caller with a
                // correct screenshot and the shared path populated. `clear`
                // handles that on the next capture, so it must not fail this one.
                if let Err(e) = std::fs::remove_file(&self.source) {
                    tracing::warn!(
                        source = %self.source.display(),
                        error = %e,
                        "copied the screenshot out but could not remove the source",
                    );
                }
                Ok(bytes)
            }
            Err(e) => Err(e),
        }
    }
}

/// Why a capture did not produce a screenshot.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The destination was not absolute. Rejected before anything is touched:
    /// the core runs as a systemd unit whose working directory is not the
    /// caller's, so a relative path would resolve somewhere neither meant.
    #[error("screenshot destination must be an absolute path, got {dest:?}")]
    RelativeDestination { dest: String },
    /// Rule 2's guard failed. See [`CaptureFile::clear`].
    #[error(
        "could not clear {path} before requesting a screenshot: {source}; refusing to \
         continue, because a file found there afterwards could be the previous capture"
    )]
    Clear {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("requesting a screenshot: {0}")]
    Request(#[source] AtomError),
    #[error("reading back the screenshot request: {0}")]
    Poll(#[source] AtomError),
    /// The request property was still set at the deadline: the compositor never
    /// picked the request up, or never reached a repaint.
    #[error(
        "the compositor did not complete a screenshot within {waited_ms} ms \
         (bound {bound_ms} ms); {atom} is still set on the root"
    )]
    TimedOut {
        waited_ms: u64,
        bound_ms: u64,
        atom: &'static str,
    },
    /// Rule 1. The attempt finished and produced nothing.
    #[error(
        "the compositor finished the screenshot request but wrote no file to {path}; \
         gamescope clears the request property on failure as well as on success, so \
         this is a failed capture, not a slow one"
    )]
    NotWritten { path: String },
    #[error("moving the screenshot to {dest}: {source}")]
    Harvest {
        dest: String,
        #[source]
        source: std::io::Error,
    },
}

/// A capture that happened, with the numbers that say how well.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Captured {
    /// Where the screenshot now is — the caller's destination, absolute.
    pub path: String,
    /// Its size on disk, so a caller has something to check that is not merely
    /// "the file exists".
    pub bytes: u64,
    /// Milliseconds from the request write to the file landing at `path`.
    /// Returned rather than dropped because [`DEFAULT_SCREENSHOT_TIMEOUT`] is a
    /// guess and this is the measurement that would replace it.
    pub took_ms: u64,
}

/// Capture the screen to `dest`.
///
/// **Callers must serialize this against other captures.** The compositor holds
/// exactly one pending request (`m_ScreenshotInfo` is a single `std::optional`,
/// so a second request overwrites the first) and writes to one shared path, so
/// two concurrent captures race for both. `GamescopeCompositor` wraps every call
/// in a gate of its own.
pub fn capture(
    surface: &impl ScreenshotSurface,
    file: &impl CaptureFile,
    dest: &str,
    timeout: Duration,
) -> Result<Captured, CaptureError> {
    capture_with(surface, file, dest, timeout, || {
        std::thread::sleep(POLL_INTERVAL)
    })
}

/// [`capture`] with the inter-poll wait injected, so tests need not sleep.
fn capture_with(
    surface: &impl ScreenshotSurface,
    file: &impl CaptureFile,
    dest: &str,
    timeout: Duration,
    mut wait: impl FnMut(),
) -> Result<Captured, CaptureError> {
    let path = Path::new(dest);
    if !path.is_absolute() {
        return Err(CaptureError::RelativeDestination {
            dest: dest.to_string(),
        });
    }

    // Rule 2, and it must come BEFORE the request: clearing afterwards would
    // delete the very frame we asked for.
    file.clear().map_err(|source| CaptureError::Clear {
        path: GAMESCOPE_OUTPUT.to_string(),
        source,
    })?;

    let started = Instant::now();
    // ONE write. gamescope composites on its next repaint.
    surface
        .request(FULL_COMPOSITION)
        .map_err(CaptureError::Request)?;

    // Wait for the compositor to stop working on it. Per Rule 1, this loop
    // learns only that the attempt ENDED.
    loop {
        if !surface.outstanding().map_err(CaptureError::Poll)? {
            break;
        }
        if started.elapsed() >= timeout {
            return Err(CaptureError::TimedOut {
                waited_ms: elapsed_ms(started),
                bound_ms: timeout.as_millis() as u64,
                atom: crate::atoms::names::REQUEST_SCREENSHOT,
            });
        }
        wait();
    }

    // Rule 1's other half: whether it WORKED is a separate question, answered on
    // disk. Reading the answer as "this request's frame" is sound only because
    // of Rule 2 — the path was empty when we asked. gamescope deletes the
    // property after closing the file, so there is no window in which the
    // property is gone and the file is still being written.
    if !file.present() {
        return Err(CaptureError::NotWritten {
            path: GAMESCOPE_OUTPUT.to_string(),
        });
    }

    let bytes = file.take(path).map_err(|source| CaptureError::Harvest {
        dest: dest.to_string(),
        source,
    })?;

    Ok(Captured {
        path: dest.to_string(),
        bytes,
        took_ms: elapsed_ms(started),
    })
}

fn elapsed_ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A stand-in for the shared `/tmp/gamescope.png`, as a presence flag plus a
    /// GENERATION — which capture wrote what is currently there. Generation `0`
    /// is a leftover from before this test's request; `1` is this request's own
    /// frame. Without that distinction a test cannot tell a stale screenshot
    /// from a fresh one, which is the entire subject of Rule 2.
    #[derive(Default)]
    struct FakeFile {
        present: RefCell<bool>,
        generation: RefCell<u32>,
        clear_fails: RefCell<bool>,
        take_fails: RefCell<bool>,
        cleared: RefCell<u32>,
        /// Set by `take` to the generation it harvested.
        taken_generation: RefCell<Option<u32>>,
    }

    impl FakeFile {
        /// A file left over from a PREVIOUS capture — generation 0.
        fn with_stale_leftover() -> Rc<Self> {
            let f = Rc::new(Self::default());
            *f.present.borrow_mut() = true;
            *f.generation.borrow_mut() = 0;
            f
        }
    }

    impl CaptureFile for Rc<FakeFile> {
        fn clear(&self) -> std::io::Result<()> {
            if *self.clear_fails.borrow() {
                return Err(std::io::Error::other("permission denied"));
            }
            *self.cleared.borrow_mut() += 1;
            *self.present.borrow_mut() = false;
            Ok(())
        }
        fn present(&self) -> bool {
            *self.present.borrow()
        }
        fn take(&self, _dest: &Path) -> std::io::Result<u64> {
            if *self.take_fails.borrow() {
                return Err(std::io::Error::other("no such directory"));
            }
            *self.taken_generation.borrow_mut() = Some(*self.generation.borrow());
            *self.present.borrow_mut() = false;
            Ok(4096)
        }
    }

    /// A compositor that finishes after `polls_until_done` polls, and — when
    /// `writes_on_finish` — writes a generation-1 frame as it does.
    struct FakeSurface {
        polls_until_done: RefCell<u32>,
        writes_on_finish: bool,
        file: Rc<FakeFile>,
        request_fails: bool,
        poll_fails: bool,
        requested: RefCell<Vec<u32>>,
    }

    impl ScreenshotSurface for FakeSurface {
        fn request(&self, kind: u32) -> Result<(), AtomError> {
            if self.request_fails {
                return Err(AtomError::Connect("no display".into()));
            }
            self.requested.borrow_mut().push(kind);
            Ok(())
        }
        fn outstanding(&self) -> Result<bool, AtomError> {
            if self.poll_fails {
                return Err(AtomError::Connect("connection lost".into()));
            }
            let mut left = self.polls_until_done.borrow_mut();
            if *left == 0 {
                // The attempt has ended. Whether it produced anything is the
                // separate question, which this fake answers either way.
                if self.writes_on_finish && !*self.file.present.borrow() {
                    *self.file.present.borrow_mut() = true;
                    *self.file.generation.borrow_mut() = 1;
                }
                return Ok(false);
            }
            *left -= 1;
            Ok(true)
        }
    }

    fn surface(file: &Rc<FakeFile>, polls: u32, writes_on_finish: bool) -> FakeSurface {
        FakeSurface {
            polls_until_done: RefCell::new(polls),
            writes_on_finish,
            file: Rc::clone(file),
            request_fails: false,
            poll_fails: false,
            requested: RefCell::new(Vec::new()),
        }
    }

    fn run(
        s: &FakeSurface,
        f: &Rc<FakeFile>,
        dest: &str,
        timeout: Duration,
    ) -> Result<Captured, CaptureError> {
        capture_with(s, f, dest, timeout, || {})
    }

    // -- the happy path ------------------------------------------------------

    #[test]
    fn a_completed_capture_returns_the_destination_and_its_size() {
        let f = Rc::new(FakeFile::default());
        let s = surface(&f, 3, true);
        let got = run(&s, &f, "/tmp/shot.png", Duration::from_secs(5)).unwrap();
        assert_eq!(got.path, "/tmp/shot.png");
        assert_eq!(got.bytes, 4096);
        // The type asked for is the one the module argues for, and no other.
        assert_eq!(*s.requested.borrow(), vec![FULL_COMPOSITION]);
        assert_eq!(
            FULL_COMPOSITION, 3,
            "gamescope-control.xml full_composition is 3"
        );
    }

    // -- Rule 1: a cleared property is not success ---------------------------

    /// **Rule 1: a finished attempt that wrote no file is an error, never a
    /// screenshot.**
    ///
    /// gamescope deletes the request property on its write-failure path
    /// (`steamcompmgr.cpp:3316`) exactly as it does on success, so "the property
    /// is gone" is worth nothing on its own.
    ///
    /// Mutation-check (run 2026-09-08): delete the `if !file.present()` guard in
    /// `capture_with` and this fails with `Ok(Captured { .. })` — a reported
    /// screenshot for a capture that produced no file.
    #[test]
    fn a_finished_request_that_produced_no_file_is_not_a_screenshot() {
        let f = Rc::new(FakeFile::default());
        let s = surface(&f, 2, false);
        let err = run(&s, &f, "/tmp/shot.png", Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, CaptureError::NotWritten { .. }), "{err:?}");
        // The message must say why a cleared property is not enough — that is
        // the whole non-obvious part, and the next reader needs it.
        assert!(
            err.to_string().contains("clears the request property"),
            "{err}"
        );
    }

    // -- Rule 2: a stale screenshot is unrepresentable -----------------------

    /// **Rule 2: the shared output path is cleared BEFORE the request, so a
    /// failed capture cannot hand back the previous one.**
    ///
    /// This is the defect the fixed path invites: `/tmp/gamescope.png` still
    /// holds the last successful capture — a valid PNG, right size, wrong
    /// moment. An agent would "verify" a change that never rendered.
    ///
    /// Mutation-check (run 2026-09-08): remove the `file.clear()` call from
    /// `capture_with` and this fails at `matches!(err, NotWritten)` — the
    /// leftover is still present, so the capture "succeeds" and harvests
    /// generation 0. That mutation takes three tests down together — this one,
    /// `a_successful_capture_over_a_leftover_returns_the_new_frame` and
    /// `a_failed_clear_refuses_the_capture_rather_than_risking_a_stale_frame` —
    /// which is the point of having all three: the last of them pins the
    /// weaker mutation (`?` → `let _ =`) that only IT catches.
    #[test]
    fn a_leftover_screenshot_is_never_returned_as_this_captures_result() {
        let f = FakeFile::with_stale_leftover();
        // The compositor finishes the attempt and writes NOTHING.
        let s = surface(&f, 1, false);
        let err = run(&s, &f, "/tmp/shot.png", Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, CaptureError::NotWritten { .. }), "{err:?}");
        assert_eq!(
            *f.cleared.borrow(),
            1,
            "the output path must be cleared exactly once, before the request"
        );
        assert!(
            f.taken_generation.borrow().is_none(),
            "nothing may be harvested from a capture that produced no frame",
        );
    }

    /// The same setup, but the compositor DOES capture: the caller gets this
    /// request's frame (generation 1), not the leftover (generation 0).
    ///
    /// Without this, the test above could be satisfied by a `capture` that
    /// always fails.
    #[test]
    fn a_successful_capture_over_a_leftover_returns_the_new_frame() {
        let f = FakeFile::with_stale_leftover();
        let s = surface(&f, 2, true);
        let got = run(&s, &f, "/tmp/shot.png", Duration::from_secs(5)).unwrap();
        assert_eq!(got.bytes, 4096);
        assert_eq!(
            *f.taken_generation.borrow(),
            Some(1),
            "the harvested frame must be the one this request produced",
        );
    }

    /// **A `clear` that fails aborts the capture** rather than proceeding and
    /// hoping. After a failed clear, nothing downstream can tell this request's
    /// frame from the last one's — the exact state Rule 2 exists to prevent.
    ///
    /// Mutation-check (run 2026-09-08): change the `?` on `file.clear()` to a
    /// `let _ =` and this fails with `Ok(Captured { .. })`, having harvested
    /// generation 0 — the stale frame, reported as a fresh screenshot. That is
    /// the bug this test is for, and this is the ONLY test that catches that
    /// mutation: the run took exactly one test down.
    #[test]
    fn a_failed_clear_refuses_the_capture_rather_than_risking_a_stale_frame() {
        let f = FakeFile::with_stale_leftover();
        *f.clear_fails.borrow_mut() = true;
        let s = surface(&f, 0, false);
        let err = run(&s, &f, "/tmp/shot.png", Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, CaptureError::Clear { .. }), "{err:?}");
        assert!(
            s.requested.borrow().is_empty(),
            "nothing may be requested once the guard has failed",
        );
        assert!(f.taken_generation.borrow().is_none());
    }

    // -- bounds and error propagation ----------------------------------------

    /// A compositor that never finishes is a timeout — not a hang, and not an
    /// `ok`. The error names the atom so an operator can check it by hand.
    #[test]
    fn a_request_that_never_clears_times_out() {
        let f = Rc::new(FakeFile::default());
        let s = surface(&f, u32::MAX, false);
        let err = run(&s, &f, "/tmp/shot.png", Duration::from_millis(0)).unwrap_err();
        match err {
            CaptureError::TimedOut { bound_ms, atom, .. } => {
                assert_eq!(bound_ms, 0);
                assert_eq!(atom, "GAMESCOPECTRL_REQUEST_SCREENSHOT");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A timeout must not then harvest whatever happens to be on disk. The
    /// belt-and-braces case: `clear` succeeded, and something else populated the
    /// shared path while we were waiting.
    #[test]
    fn a_timeout_never_harvests_whatever_is_on_the_shared_path() {
        let f = FakeFile::with_stale_leftover();
        let s = surface(&f, u32::MAX, false);
        *f.present.borrow_mut() = true;
        let err = run(&s, &f, "/tmp/shot.png", Duration::from_millis(0)).unwrap_err();
        assert!(matches!(err, CaptureError::TimedOut { .. }), "{err:?}");
        assert!(f.taken_generation.borrow().is_none());
    }

    #[test]
    fn a_relative_destination_is_rejected_before_anything_is_touched() {
        let f = FakeFile::with_stale_leftover();
        let s = surface(&f, 0, true);
        let err = run(&s, &f, "shot.png", Duration::from_secs(5)).unwrap_err();
        assert!(
            matches!(err, CaptureError::RelativeDestination { .. }),
            "{err:?}"
        );
        // Nothing may have happened yet: not the clear, not the request.
        assert_eq!(*f.cleared.borrow(), 0);
        assert!(s.requested.borrow().is_empty());
        assert!(f.present(), "the leftover must be left alone");
    }

    #[test]
    fn a_failed_request_write_is_an_error() {
        let f = Rc::new(FakeFile::default());
        let mut s = surface(&f, 0, true);
        s.request_fails = true;
        let err = run(&s, &f, "/tmp/shot.png", Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, CaptureError::Request(_)), "{err:?}");
    }

    #[test]
    fn a_failed_poll_is_an_error_not_a_finished_attempt() {
        let f = Rc::new(FakeFile::default());
        let mut s = surface(&f, 5, true);
        s.poll_fails = true;
        let err = run(&s, &f, "/tmp/shot.png", Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, CaptureError::Poll(_)), "{err:?}");
    }

    /// A capture that happened but could not be moved is an error naming the
    /// destination: the operator's problem is a bad path, and the message must
    /// say so rather than reporting a mysterious failed screenshot.
    #[test]
    fn a_failed_harvest_names_the_destination() {
        let f = Rc::new(FakeFile::default());
        let s = surface(&f, 1, true);
        *f.take_fails.borrow_mut() = true;
        let err = run(&s, &f, "/nope/shot.png", Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, CaptureError::Harvest { .. }), "{err:?}");
        assert!(err.to_string().contains("/nope/shot.png"), "{err}");
    }

    /// Every failure must survive the newline-framed wire on one line. The
    /// `Clear` and `NotWritten` messages are the long, multi-clause ones.
    #[test]
    fn every_error_message_stays_on_one_line() {
        let errors = [
            CaptureError::RelativeDestination {
                dest: "a\nb".into(),
            },
            CaptureError::Clear {
                path: GAMESCOPE_OUTPUT.into(),
                source: std::io::Error::other("denied"),
            },
            CaptureError::TimedOut {
                waited_ms: 1,
                bound_ms: 2,
                atom: "X",
            },
            CaptureError::NotWritten {
                path: GAMESCOPE_OUTPUT.into(),
            },
            CaptureError::Harvest {
                dest: "/tmp/x.png".into(),
                source: std::io::Error::other("denied"),
            },
        ];
        for e in errors {
            let line = crate::protocol::resp_error(&e.to_string());
            assert!(!line.contains('\n'), "{line:?}");
            assert!(line.starts_with("error:"), "{line}");
        }
    }

    // -- the real filesystem half --------------------------------------------

    /// A unique scratch FILE path directly under `/tmp` — deliberately not a
    /// scratch directory, and deliberately not `std::env::temp_dir()`.
    ///
    /// **No directory**, because `ipc::bind` sets `umask(0o177)`
    /// process-globally around its `bind()`. That umask is documented there as
    /// failing closed — "over-restrictive permissions on an unrelated file,
    /// never over-permissive ones" — which is true about leaking and false
    /// about harmlessness: a *directory* created inside that window loses its
    /// `x` bit, and nothing can then be unlinked inside it. These tests
    /// originally made a scratch dir and failed `EACCES` on roughly one run in
    /// four, in a module that touches no permissions at all. A plain file under
    /// the already-`1777` `/tmp` is immune: 0600 is perfectly usable by its
    /// owner.
    ///
    /// **No `temp_dir()`**, because it reads `TMPDIR`, and an environment read
    /// concurrent with this crate's `set_var` tests is unsound on its own (see
    /// `crate::ENV_GUARD`). Not the cause of the flake above, but the same kind
    /// of ambient dependency, and just as easy not to have.
    fn scratch(tag: &str) -> PathBuf {
        let p = PathBuf::from("/tmp").join(format!("tv-core-shot-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// `clear` treats an absent file as success: the first capture after a boot
    /// has nothing to remove, and making that an error would refuse every cold
    /// capture on the box.
    #[test]
    fn clearing_an_absent_output_is_success() {
        let out = GamescopeOutput {
            source: scratch("absent"),
        };
        out.clear().unwrap();
        assert!(!out.present());
    }

    /// `clear` really removes an existing file — the operative half of Rule 2 on
    /// the real filesystem.
    #[test]
    fn clearing_an_existing_output_removes_it() {
        let source = scratch("clear");
        std::fs::write(&source, b"OLD").unwrap();
        let out = GamescopeOutput {
            source: source.clone(),
        };
        assert!(out.present());
        out.clear().unwrap();
        assert!(!out.present());
        assert!(!source.exists());
    }

    /// `take` moves rather than copies: the shared path must be empty
    /// afterwards, so a later failed capture has nothing to find there.
    #[test]
    fn taking_the_output_moves_it_off_the_shared_path() {
        let source = scratch("mv-src");
        let dest = scratch("mv-dest");
        std::fs::write(&source, b"PNGDATA").unwrap();
        let out = GamescopeOutput {
            source: source.clone(),
        };
        assert_eq!(out.take(&dest).unwrap(), 7);
        assert!(
            !out.present(),
            "the shared path must be empty after a harvest"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"PNGDATA");
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn the_default_output_is_the_path_gamescope_hardcodes() {
        assert_eq!(
            GamescopeOutput::default().source,
            Path::new("/tmp/gamescope.png")
        );
        assert_eq!(GAMESCOPE_OUTPUT, "/tmp/gamescope.png");
    }
}
