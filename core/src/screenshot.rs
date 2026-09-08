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
//! observed on the box, in a probe run that was **not** authorised and has not
//! been repeated under controlled conditions; the conclusion is corroborated by
//! the gamescope and wlroots sources (`-rootless` is passed unconditionally, and
//! `steamcompmgr` redirects manually), which is why it is stated here as fact.
//! And the v2 shell is not an X client in the first place:
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
//! 3. **gamescope deletes the property well BEFORE the file exists.** Measured
//!    on the deploy box, live v2 session: the property cleared at **32 ms** and
//!    the file appeared at **712 ms** — a 22x gap, and the poll that saw the
//!    property already gone saw the file still absent in the same iteration.
//!
//! Point 3 replaces an earlier claim in this module that the delete happened
//! *after* the file was closed, so no such window existed. That claim was read
//! off `steamcompmgr.cpp:3295` and it is **wrong in practice**; the measurement
//! is the authority. It is called out rather than quietly corrected because the
//! bug it caused — waiting on the property and then harvesting — is the exact
//! shape this module is supposed to prevent, and reasoning from the source
//! instead of from the hardware is how it got in.
//!
//! # The two rules, and why they are rules
//!
//! **Rule 1 — the file is the completion signal; the property is a
//! diagnostic.** The property clearing does not mean the capture succeeded
//! (gamescope clears it on the write-FAILURE path too, `:3316`) and, per point
//! 3, it does not even mean the capture *finished*. Exiting the wait on it and
//! then checking the file reports `NotWritten` roughly 680 ms into a capture
//! that goes on to succeed — a false failure, and the mirror image of the stale
//! frame Rule 2 exists to stop. So the wait is satisfied by a **complete file**
//! and by nothing else, the timeout governs that wait, and the property is
//! polled only to record *when* it cleared, which sharpens the error when one is
//! returned. A capture that never produces a file still fails; it just takes the
//! full deadline to say so, because until the deadline "failed" and "slow" are
//! genuinely indistinguishable.
//!
//! **Complete, not merely present.** gamescope writes the PNG in place with
//! `stbi_write_png` — there is no write-then-rename — so a reader can observe a
//! partial file. "Present" is therefore not a completion test: a frame counts
//! only when it opens with the PNG signature and closes with an `IEND` chunk.
//! That is a structural check rather than a size-stability heuristic, which
//! cannot tell a finished small file from a stalled large one.
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
//!   dumps the raw PQ scanout buffer into an 8-bit sRGB PNG, which on an HDR
//!   session is visibly washed out with lifted blacks (observed once, in the
//!   same unauthorised probe as above, and consistent with the code path). A
//!   knob whose other positions are all defects is not a knob.
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
/// **A guess. There is no measured number behind it yet.** gamescope stores the
/// request and raises `hasRepaint`, but — unlike `force_repaint` — it does
/// **not** call `nudge_steamcompmgr()`, so the capture waits for the next
/// vblank-gated repaint and then for a detached thread to encode and write the
/// PNG. That is an argument for "not instant", not a duration.
///
/// Five seconds is chosen to be long enough that a loaded box cannot produce a
/// false failure and short enough that a wedged compositor says so inside one
/// interaction. [`Captured::took_ms`] exists to replace it with a real figure
/// once one is taken under controlled conditions; until then, treat this as
/// unvalidated and do not quote it as a bound anyone has checked.
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
    /// Is the request property still set?
    ///
    /// **Diagnostic only — never a completion signal.** It was measured
    /// clearing 680 ms before the file existed, so `false` here means neither
    /// "the capture worked" nor even "the capture is over". [`capture`] uses it
    /// solely to record when the compositor let go of the request.
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
    /// Is a **complete** frame sitting at the output path?
    ///
    /// Not "does the file exist". gamescope writes the PNG in place with
    /// `stbi_write_png` and never renames, so a poll can land mid-write and see
    /// a real file that is half a picture. This is the wait loop's exit
    /// condition, so a partial file passing it would be harvested and handed to
    /// the caller as a screenshot.
    fn complete_frame(&self) -> bool;
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

    fn complete_frame(&self) -> bool {
        complete_png(&self.source).unwrap_or(false)
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

/// The 8-byte PNG signature every PNG opens with.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// The final chunk of a complete PNG: a zero length, the `IEND` type, and that
/// chunk's (constant) CRC32. A file ending in these twelve bytes has had its
/// last chunk written.
const PNG_IEND: [u8; 12] = [
    0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
];

/// Is `path` a PNG that has been written all the way to its end?
///
/// Structural rather than heuristic: the signature says it is a PNG at all, and
/// the trailing `IEND` says the writer got to the end. The alternative —
/// watching the size hold steady across two polls — cannot tell a finished small
/// file from a stalled large one, and would pass a truncated frame whenever the
/// writer happened to be descheduled between polls.
///
/// Any I/O error is "not complete": a file being written can legitimately fail a
/// read, and the caller's deadline is what turns a persistent failure into an
/// error. Cheap enough to poll — it reads twenty bytes, not the image.
fn complete_png(path: &Path) -> std::io::Result<bool> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len < (PNG_SIGNATURE.len() + PNG_IEND.len()) as u64 {
        return Ok(false);
    }
    let mut head = [0u8; PNG_SIGNATURE.len()];
    f.read_exact(&mut head)?;
    if head != PNG_SIGNATURE {
        return Ok(false);
    }
    f.seek(SeekFrom::End(-(PNG_IEND.len() as i64)))?;
    let mut tail = [0u8; PNG_IEND.len()];
    f.read_exact(&mut tail)?;
    Ok(tail == PNG_IEND)
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
    /// No frame, and the compositor never even let go of the request: it did not
    /// pick the request up, or never reached a repaint. Usually a wedged or dead
    /// compositor rather than a failed capture.
    #[error(
        "no screenshot within {waited_ms} ms (bound {bound_ms} ms), and {atom} is STILL \
         set on the root — the compositor never picked the request up"
    )]
    TimedOut {
        waited_ms: u64,
        bound_ms: u64,
        atom: &'static str,
    },
    /// The compositor took the request and no complete frame ever appeared.
    ///
    /// `cleared_after_ms` is the diagnostic that separates this from
    /// [`Self::TimedOut`]: the request WAS picked up. Note it is normal for that
    /// number to be a small fraction of `waited_ms` — the property clears long
    /// before the file lands (32 ms vs 712 ms, measured) — so a clear followed
    /// by a file is the healthy case, not this one.
    #[error(
        "the compositor took the screenshot request (cleared after {cleared_after_ms} ms) \
         but no complete PNG appeared at {path} within {waited_ms} ms (bound {bound_ms} ms); \
         the request property clears long before the file is written, so this waited for \
         the FILE and it never arrived"
    )]
    NotWritten {
        path: String,
        cleared_after_ms: u64,
        waited_ms: u64,
        bound_ms: u64,
    },
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
    /// Milliseconds from the request write to a COMPLETE file at the output
    /// path. Returned rather than dropped because
    /// [`DEFAULT_SCREENSHOT_TIMEOUT`] is a guess and this is the measurement
    /// that would replace it.
    pub took_ms: u64,
    /// Milliseconds from the request write to the compositor clearing the
    /// request property, when that was observed before the file landed.
    ///
    /// Reported because the gap between this and [`Self::took_ms`] is the whole
    /// reason the wait is on the file: they were measured 32 ms and 712 ms
    /// apart. A deployment where they converge is one where the old
    /// property-based wait would have looked fine, and is worth knowing about.
    /// `None` means the file beat the first property poll.
    pub request_cleared_ms: Option<u64>,
}

/// Capture the screen to `dest`.
///
/// **Callers must serialize this against other captures.** The compositor holds
/// exactly one pending request (`m_ScreenshotInfo` is a single `std::optional`,
/// so a second request overwrites the first) and writes to one shared path, so
/// two concurrent captures race for both. `GamescopeCompositor` wraps every call
/// in a gate of its own.
///
/// That gate is process-local, and it is the limit of what this can promise.
/// Another process on the box capturing at the same moment — `gamescopectl
/// screenshot`, a `SIGUSR2`, the built-in keybind — shares the compositor's one
/// request slot, and gamescope deletes BOTH request atoms when any capture
/// finishes, so their completion can end our wait. The frame we then harvest is
/// still a real full-screen capture taken a few milliseconds either side of the
/// one we asked for, so it is not the stale-frame failure Rule 2 exists to stop;
/// it is simply not provably *our* frame. Making it provable needs a lock
/// outside this process, and nothing else on this box takes screenshots today.
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

    // **Wait for the FILE, not for the property** (Rule 1). The property was
    // measured clearing at 32 ms against a file that landed at 712 ms, so
    // breaking on the property and then testing the file reports `NotWritten`
    // most of a second into a capture that succeeds.
    //
    // Reading a complete file here as "this request's frame" is sound only
    // because of Rule 2: the path was empty when we asked.
    let mut cleared_after_ms: Option<u64> = None;
    loop {
        if file.complete_frame() {
            break;
        }
        // Diagnostic only, and only until it happens: once the compositor has
        // let go of the request there is nothing further to learn, so this stops
        // costing an X round trip per poll.
        if cleared_after_ms.is_none() && !surface.outstanding().map_err(CaptureError::Poll)? {
            cleared_after_ms = Some(elapsed_ms(started));
        }
        if started.elapsed() >= timeout {
            let waited_ms = elapsed_ms(started);
            let bound_ms = timeout.as_millis() as u64;
            // Which failure it is turns on whether the compositor ever took the
            // request — the one thing the property is good for.
            return Err(match cleared_after_ms {
                Some(cleared_after_ms) => CaptureError::NotWritten {
                    path: GAMESCOPE_OUTPUT.to_string(),
                    cleared_after_ms,
                    waited_ms,
                    bound_ms,
                },
                None => CaptureError::TimedOut {
                    waited_ms,
                    bound_ms,
                    atom: crate::atoms::names::REQUEST_SCREENSHOT,
                },
            });
        }
        wait();
    }

    let bytes = file.take(path).map_err(|source| CaptureError::Harvest {
        dest: dest.to_string(),
        source,
    })?;

    Ok(Captured {
        path: dest.to_string(),
        bytes,
        took_ms: elapsed_ms(started),
        request_cleared_ms: cleared_after_ms,
    })
}

fn elapsed_ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// A poll counter shared by both fakes, incremented by the injected wait.
    ///
    /// **The compositor clearing the request and the file appearing are
    /// scheduled INDEPENDENTLY against this clock, and that is the point.** The
    /// first version of these fakes had the surface write the file at the moment
    /// it cleared the property — it encoded the assumption the code was built
    /// on. The suite was green and the implementation was broken on real
    /// hardware, where the property cleared at 32 ms and the file landed at
    /// 712 ms. A fake that can only produce the timeline you expect cannot
    /// falsify anything, so here the two events are set separately and the
    /// interesting cases are the ones where they disagree.
    type Clock = Rc<Cell<u32>>;

    /// A stand-in for the shared `/tmp/gamescope.png`.
    ///
    /// Carries a GENERATION as well as presence — `0` is a leftover from before
    /// this request, `1` is this request's own frame — because without it a test
    /// cannot tell a stale screenshot from a fresh one, which is the whole
    /// subject of Rule 2.
    struct FakeFile {
        clock: Clock,
        present: RefCell<bool>,
        generation: RefCell<u32>,
        /// The poll at which this request's frame lands. `None` = never.
        appears_at: RefCell<Option<u32>>,
        /// The frame that lands is a PARTIAL png: present, but not complete.
        truncated: RefCell<bool>,
        clear_fails: RefCell<bool>,
        take_fails: RefCell<bool>,
        cleared: RefCell<u32>,
        /// Set by `take` to the generation it harvested.
        taken_generation: RefCell<Option<u32>>,
    }

    impl FakeFile {
        fn new(clock: &Clock, appears_at: Option<u32>) -> Rc<Self> {
            Rc::new(Self {
                clock: Rc::clone(clock),
                present: RefCell::new(false),
                generation: RefCell::new(0),
                appears_at: RefCell::new(appears_at),
                truncated: RefCell::new(false),
                clear_fails: RefCell::new(false),
                take_fails: RefCell::new(false),
                cleared: RefCell::new(0),
                taken_generation: RefCell::new(None),
            })
        }

        /// As `new`, but with a file left over from a PREVIOUS capture —
        /// generation 0, present before this request is even made.
        fn with_stale_leftover(clock: &Clock, appears_at: Option<u32>) -> Rc<Self> {
            let f = Self::new(clock, appears_at);
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

        fn complete_frame(&self) -> bool {
            // This request's frame lands at its own scheduled poll, whatever the
            // compositor has done with the request property.
            if let Some(at) = *self.appears_at.borrow() {
                if self.clock.get() >= at && !*self.present.borrow() {
                    *self.present.borrow_mut() = true;
                    *self.generation.borrow_mut() = 1;
                }
            }
            *self.present.borrow() && !*self.truncated.borrow()
        }

        fn take(&self, _dest: &Path) -> std::io::Result<u64> {
            if *self.take_fails.borrow() {
                return Err(std::io::Error::other("no such directory"));
            }
            // Harvesting something that is not there is the shape a premature
            // exit from the wait loop produces, so it fails the way the real
            // filesystem would rather than silently reporting bytes.
            if !*self.present.borrow() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no such file",
                ));
            }
            *self.taken_generation.borrow_mut() = Some(*self.generation.borrow());
            *self.present.borrow_mut() = false;
            Ok(4096)
        }
    }

    /// A compositor that lets go of the request property at a given poll.
    struct FakeSurface {
        clock: Clock,
        /// The poll at which the request property disappears.
        clears_at: u32,
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
            Ok(self.clock.get() < self.clears_at)
        }
    }

    fn surface(clock: &Clock, clears_at: u32) -> FakeSurface {
        FakeSurface {
            clock: Rc::clone(clock),
            clears_at,
            request_fails: false,
            poll_fails: false,
            requested: RefCell::new(Vec::new()),
        }
    }

    /// Never clears — a compositor that did not pick the request up.
    const NEVER: u32 = u32::MAX;

    fn run(
        s: &FakeSurface,
        f: &Rc<FakeFile>,
        clock: &Clock,
        dest: &str,
        timeout: Duration,
    ) -> Result<Captured, CaptureError> {
        let clock = Rc::clone(clock);
        capture_with(s, f, dest, timeout, move || clock.set(clock.get() + 1))
    }

    /// Long enough that the wall-clock deadline never fires: these tests are
    /// driven by the poll clock, not by real time.
    fn generous() -> Duration {
        Duration::from_secs(60)
    }

    // -- the happy path ------------------------------------------------------

    #[test]
    fn a_completed_capture_returns_the_destination_and_its_size() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::new(&clock, Some(3));
        let s = surface(&clock, 1);
        let got = run(&s, &f, &clock, "/tmp/shot.png", generous()).unwrap();
        assert_eq!(got.path, "/tmp/shot.png");
        assert_eq!(got.bytes, 4096);
        // The type asked for is the one the module argues for, and no other.
        assert_eq!(*s.requested.borrow(), vec![FULL_COMPOSITION]);
        assert_eq!(
            FULL_COMPOSITION, 3,
            "gamescope-control.xml full_composition is 3"
        );
    }

    // -- Rule 1: the file is the completion signal, not the property ---------

    /// **THE REGRESSION TEST. The wait is satisfied by the FILE, and a request
    /// property that clears long before the file is not a failure.**
    ///
    /// This is the timeline measured on hardware — property cleared at 32 ms,
    /// complete PNG at 712 ms — expressed on the poll clock: the compositor lets
    /// go at poll 1, the frame lands at poll 5. The capture must SUCCEED.
    ///
    /// The original implementation broke out of the wait on the property and
    /// then tested the file, so it returned `NotWritten` at poll 1 for a capture
    /// that was working perfectly. Its fake wrote the file at the instant the
    /// property cleared, so the two could never disagree and the suite stayed
    /// green over the bug.
    ///
    /// Mutation-check (run 2026-09-08): add `if !surface.outstanding()? { break; }`
    /// back into the loop ahead of the file check and this fails —
    /// `Err(Harvest { .. })`, harvesting a file that is not there yet.
    #[test]
    fn a_property_that_clears_long_before_the_file_is_not_a_failure() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::new(&clock, Some(5));
        let s = surface(&clock, 1);
        let got = run(&s, &f, &clock, "/tmp/shot.png", generous())
            .expect("a capture whose file lands after the property clears must succeed");
        assert_eq!(got.bytes, 4096);
        assert_eq!(
            *f.taken_generation.borrow(),
            Some(1),
            "the harvested frame must be this request's own"
        );
        // The gap is reported, because it is the number that disproved the old
        // premise and the one that says whether a deployment still has it.
        assert_eq!(got.request_cleared_ms, Some(0), "measured on a fake clock");
        assert!(
            clock.get() >= 5,
            "the wait must have run to the frame's arrival, not stopped at the clear"
        );
    }

    /// **Rule 1: a capture that never produces a file is an error — but only
    /// after the deadline, never at the moment the property clears.**
    ///
    /// gamescope deletes the request property on its write-failure path
    /// (`steamcompmgr.cpp:3316`) exactly as it does on success, so a cleared
    /// property distinguishes nothing on its own. The error names when the
    /// clear happened, which is what separates this from `TimedOut`.
    #[test]
    fn a_capture_that_never_writes_a_file_fails_once_the_deadline_passes() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::new(&clock, None);
        let s = surface(&clock, 0); // already cleared: the request was taken
        let err = run(&s, &f, &clock, "/tmp/shot.png", Duration::ZERO).unwrap_err();
        match &err {
            CaptureError::NotWritten { path, .. } => assert_eq!(path, GAMESCOPE_OUTPUT),
            other => panic!("{other:?}"),
        }
        // The message must explain the ordering, since that is the whole
        // non-obvious part and the next reader needs it.
        assert!(
            err.to_string().contains("clears long before the file"),
            "{err}"
        );
        assert!(f.taken_generation.borrow().is_none());
    }

    /// A compositor that never even takes the request is a different failure,
    /// and the error says so — the atom is still set, so nothing is coming.
    #[test]
    fn a_request_the_compositor_never_took_is_a_timeout_not_a_failed_write() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::new(&clock, None);
        let s = surface(&clock, NEVER);
        let err = run(&s, &f, &clock, "/tmp/shot.png", Duration::ZERO).unwrap_err();
        match err {
            CaptureError::TimedOut { bound_ms, atom, .. } => {
                assert_eq!(bound_ms, 0);
                assert_eq!(atom, "GAMESCOPECTRL_REQUEST_SCREENSHOT");
            }
            other => panic!("{other:?}"),
        }
    }

    /// **A half-written PNG is not a frame.** gamescope writes in place with no
    /// rename, so a poll can land mid-write; harvesting then would hand the
    /// caller a truncated image that is a perfectly real file.
    ///
    /// Mutation-check (run 2026-09-08): make `FakeFile::complete_frame` return
    /// bare presence (drop the `&& !truncated`) and this fails with
    /// `Ok(Captured { .. })` — a partial frame reported as a screenshot.
    #[test]
    fn a_partially_written_file_is_not_treated_as_a_frame() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::new(&clock, Some(0));
        *f.truncated.borrow_mut() = true;
        // Already cleared: the compositor took the request, so the failure is
        // "took it and wrote no COMPLETE frame", not "never took it".
        let s = surface(&clock, 0);
        let err = run(&s, &f, &clock, "/tmp/shot.png", Duration::ZERO).unwrap_err();
        assert!(matches!(err, CaptureError::NotWritten { .. }), "{err:?}");
        assert!(
            f.taken_generation.borrow().is_none(),
            "a truncated frame must never be harvested"
        );
    }

    // -- Rule 2: a stale screenshot is unrepresentable -----------------------

    /// **Rule 2: the shared output path is cleared BEFORE the request, so a
    /// failed capture cannot hand back the previous one.**
    ///
    /// `/tmp/gamescope.png` still holds the last successful capture — a valid
    /// PNG, right size, wrong moment. An agent would "verify" a change that
    /// never rendered.
    ///
    /// Mutation-check (run 2026-09-08): remove the `file.clear()` call from
    /// `capture_with` and this fails at `matches!(err, NotWritten)` — the
    /// leftover is still present, so the capture "succeeds" and harvests
    /// generation 0. That mutation takes three tests down together; the
    /// `a_failed_clear_...` one below is the only one that also catches the
    /// weaker `?` → `let _ =` mutation.
    #[test]
    fn a_leftover_screenshot_is_never_returned_as_this_captures_result() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::with_stale_leftover(&clock, None);
        let s = surface(&clock, 0);
        let err = run(&s, &f, &clock, "/tmp/shot.png", Duration::ZERO).unwrap_err();
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
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::with_stale_leftover(&clock, Some(2));
        let s = surface(&clock, 1);
        let got = run(&s, &f, &clock, "/tmp/shot.png", generous()).unwrap();
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
    /// generation 0 — the stale frame, reported as a fresh screenshot. This is
    /// the ONLY test that catches that mutation.
    #[test]
    fn a_failed_clear_refuses_the_capture_rather_than_risking_a_stale_frame() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::with_stale_leftover(&clock, None);
        *f.clear_fails.borrow_mut() = true;
        let s = surface(&clock, 0);
        let err = run(&s, &f, &clock, "/tmp/shot.png", generous()).unwrap_err();
        assert!(matches!(err, CaptureError::Clear { .. }), "{err:?}");
        assert!(
            s.requested.borrow().is_empty(),
            "nothing may be requested once the guard has failed",
        );
        assert!(f.taken_generation.borrow().is_none());
    }

    // -- bounds and error propagation ----------------------------------------

    #[test]
    fn a_relative_destination_is_rejected_before_anything_is_touched() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::with_stale_leftover(&clock, None);
        let s = surface(&clock, 0);
        let err = run(&s, &f, &clock, "shot.png", generous()).unwrap_err();
        assert!(
            matches!(err, CaptureError::RelativeDestination { .. }),
            "{err:?}"
        );
        // Nothing may have happened yet: not the clear, not the request.
        assert_eq!(*f.cleared.borrow(), 0);
        assert!(s.requested.borrow().is_empty());
        assert!(*f.present.borrow(), "the leftover must be left alone");
    }

    #[test]
    fn a_failed_request_write_is_an_error() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::new(&clock, Some(1));
        let mut s = surface(&clock, 1);
        s.request_fails = true;
        let err = run(&s, &f, &clock, "/tmp/shot.png", generous()).unwrap_err();
        assert!(matches!(err, CaptureError::Request(_)), "{err:?}");
    }

    /// A failed property read is an error, not a silently-skipped diagnostic.
    ///
    /// The property is only a diagnostic now, but an X connection that cannot be
    /// read is a broken connection, and continuing to poll a dead socket until
    /// the deadline would report a timeout for a connection failure.
    #[test]
    fn a_failed_poll_is_an_error_not_a_finished_attempt() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::new(&clock, Some(5));
        let mut s = surface(&clock, 1);
        s.poll_fails = true;
        let err = run(&s, &f, &clock, "/tmp/shot.png", generous()).unwrap_err();
        assert!(matches!(err, CaptureError::Poll(_)), "{err:?}");
    }

    /// A capture that happened but could not be moved is an error naming the
    /// destination: the operator's problem is a bad path, and the message must
    /// say so rather than reporting a mysterious failed screenshot.
    #[test]
    fn a_failed_harvest_names_the_destination() {
        let clock: Clock = Rc::new(Cell::new(0));
        let f = FakeFile::new(&clock, Some(1));
        *f.take_fails.borrow_mut() = true;
        let s = surface(&clock, 1);
        let err = run(&s, &f, &clock, "/nope/shot.png", generous()).unwrap_err();
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
                cleared_after_ms: 32,
                waited_ms: 5000,
                bound_ms: 5000,
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

    /// The smallest byte string that satisfies [`complete_png`]: signature,
    /// something in the middle, `IEND`.
    fn whole_png() -> Vec<u8> {
        let mut v = PNG_SIGNATURE.to_vec();
        v.extend_from_slice(b"....IHDR....pretend this is image data....");
        v.extend_from_slice(&PNG_IEND);
        v
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
        assert!(!out.complete_frame());
    }

    /// `clear` really removes an existing file — the operative half of Rule 2 on
    /// the real filesystem.
    #[test]
    fn clearing_an_existing_output_removes_it() {
        let source = scratch("clear");
        std::fs::write(&source, whole_png()).unwrap();
        let out = GamescopeOutput {
            source: source.clone(),
        };
        assert!(out.complete_frame());
        out.clear().unwrap();
        assert!(!out.complete_frame());
        assert!(!source.exists());
    }

    /// **A PNG missing its `IEND` is not a complete frame**, which is the real
    /// half of the truncation rule — the unit test above proves the loop honours
    /// `complete_frame`, and this proves `complete_frame` can actually tell.
    ///
    /// Mutation-check (run 2026-09-08): make `complete_png` return
    /// `Ok(len > 0)` and this fails — a half-written capture reads as finished.
    #[test]
    fn a_png_without_its_end_chunk_is_not_a_complete_frame() {
        let source = scratch("partial");
        let whole = whole_png();
        // Everything but the last byte of IEND: a real file, a real PNG header,
        // and exactly the state a poll lands in mid-write.
        std::fs::write(&source, &whole[..whole.len() - 1]).unwrap();
        let out = GamescopeOutput {
            source: source.clone(),
        };
        assert!(
            !out.complete_frame(),
            "a truncated PNG must not read as a finished frame"
        );
        // And the moment the writer finishes, it does.
        std::fs::write(&source, &whole).unwrap();
        assert!(out.complete_frame());
        let _ = std::fs::remove_file(&source);
    }

    /// Non-PNG bytes are not a frame either, however long the file is.
    #[test]
    fn a_file_that_is_not_a_png_is_not_a_complete_frame() {
        let source = scratch("notpng");
        std::fs::write(
            &source,
            b"this is not a png but it is long enough to be one",
        )
        .unwrap();
        let out = GamescopeOutput {
            source: source.clone(),
        };
        assert!(!out.complete_frame());
        let _ = std::fs::remove_file(&source);
    }

    /// An empty file — the very first instant of a write — is not a frame.
    #[test]
    fn an_empty_output_is_not_a_complete_frame() {
        let source = scratch("empty");
        std::fs::write(&source, b"").unwrap();
        let out = GamescopeOutput {
            source: source.clone(),
        };
        assert!(!out.complete_frame());
        let _ = std::fs::remove_file(&source);
    }

    /// `take` moves rather than copies: the shared path must be empty
    /// afterwards, so a later failed capture has nothing to find there.
    #[test]
    fn taking_the_output_moves_it_off_the_shared_path() {
        let source = scratch("mv-src");
        let dest = scratch("mv-dest");
        let png = whole_png();
        std::fs::write(&source, &png).unwrap();
        let out = GamescopeOutput {
            source: source.clone(),
        };
        assert_eq!(out.take(&dest).unwrap(), png.len() as u64);
        assert!(
            !out.complete_frame(),
            "the shared path must be empty after a harvest"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), png);
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
