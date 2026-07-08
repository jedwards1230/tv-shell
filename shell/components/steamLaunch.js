.pragma library

// Steam launch / resume / quit / Big-Picture choreography, extracted from
// HomeScreen.qml (dedup/readability — zero behaviour change). Pure functions that
// take the HomeScreen `root` (for its signals / properties / methods) and, where a
// host command is fired, the relevant SocketClient QML object. Those SocketClients
// (steamLaunchReq / steamBigPictureReq / steamQuitReq) stay declared in
// HomeScreen.qml — QML objects can't move to a .js file — and are passed in here.
// HomeScreen keeps thin forwarding wrappers so the external call surface
// (homeScreen.launchSteamGame(appid), …) is unchanged. Mirrors the .pragma library
// idiom of focusChain.js / settingsPayload.js.

// === Steam widget: LOCAL launch helpers ===
// The Steam home widget (default-disabled) shows the host's Steam library poster
// grid, but activation launches Steam LOCALLY on this machine, via the normal
// LOCAL app-launch path (appLaunchRequested → AppLifecycleManager.checkAndLaunchApp),
// landing the shell in `appRunning` with window class `steam` — kiosk fullscreen
// (class-agnostic) and StreamAudioMuter's mute-on-background (`streamClasses:
// ["steam"]`) cover it automatically. This is NOT the streaming state machine, and
// it does NOT touch the host-side `steam-launch`/`steam-bigpicture` daemon commands
// below (those stay wired to the MOONLIGHT widget, for navigating/streaming the
// GAMING HOST's Big Picture over Moonlight).
//
// Steam is a SINGLE-INSTANCE app: delivering a steam:// URL to an already-running
// instance navigates it in place but spawns NO new window, so the generic cold-start
// path (checkAndLaunchApp waiting for a new/matching window before it confirms the
// launch) can never resolve for it — the "Launching…" overlay would hang until its
// safety timeout, and the window would never get focused (checkAndLaunchApp's
// match-found branch only focuses; it doesn't redeliver the URL). So check the live
// `runningWindows` model (the same data the recent-apps "Focus" context action reads)
// FIRST: if Steam is already running, skip launchApp entirely and fire
// appResumeRequested, which redelivers the URL AND focuses the window by address in
// one call — mirroring focusByAddress, so it never flashes the launch overlay,
// exactly like resuming any other already-running app. Cold start (Steam not running
// yet) is unaffected — it still goes through the normal launchApp/checkAndLaunchApp
// flow.
function launchSteamLocalGame(root, appid) {
    root.userActivity();
    launchOrResumeSteamLocal(root, "steam steam://nav/games/details/" + appid);
}

function launchSteamLocalBigPicture(root) {
    root.userActivity();
    launchOrResumeSteamLocal(root, "steam steam://open/bigpicture");
}

function launchOrResumeSteamLocal(root, execCmd) {
    let app = {
        "name": "Steam",
        "exec": execCmd,
        "wmClass": "steam",
        "icon": "steam",
        "comment": "Steam"
    };
    let running = root.runningWindows || [];
    for (let i = 0; i < running.length; i++) {
        if ((running[i].windowClass || "").toLowerCase() === "steam") {
            root.appResumeRequested(app, running[i].address);
            return;
        }
    }
    root.launchApp(app);
}

// === Steam launch choreography ===
// Select a Steam card →
//   1. `steam-launch <appid>` → the host NAVIGATES Big Picture to that game's page
//      (it no longer launches the game directly — just moves BPM).
//   2. If THIS client is NOT already viewing a stream (`shellState !== "streaming"`)
//      → start one stream to the primary target (targets[0]). Moonlight RESUMES a
//      host that already has a session, so this both opens a fresh stream and
//      reconnects to a resumable one — selecting a game must ALWAYS get you onto the
//      screen.
//   3. If this client IS already streaming → the nav alone moved the live BPM; don't
//      launch a 2nd local Moonlight process.
// The gate is THIS client's own `shellState`, NOT the host's session flag
// (`moonlightWidget.streaming`, which is true whenever *any* client — e.g. the laptop
// — holds a resumable session). Gating on the host flag wrongly blocked
// launching/resuming whenever a session existed elsewhere; that flag drives the
// session INDICATOR only. Never trigger a `rungameid`-style direct launch.
function launchSteamGame(root, steamLaunchReq, appid) {
    root.userActivity();
    // Fire the host-side navigate (fire-and-forget; the reply is just ok/error).
    steamLaunchReq.request("steam-launch", appid);
    if (root.shellState === "streaming") {
        // This client is already in the stream — the nav moved the live BPM.
        // Don't start a 2nd local Moonlight process.
        return;
    }
    // Not viewing a stream here → start/resume one to the primary target. With no
    // target configured there's nothing to stream into; the nav still fired.
    let ts = root.targets || [];
    if (ts.length > 0)
        root.streamRequested(ts[0]);
    else
        console.log("HomeScreen: steam-launch " + appid + " sent, but no stream target configured");
}

// === Open Steam Big Picture (home) choreography ===
// The "Open Steam" action chip in the library view →
//   1. `steam-bigpicture` (no appid) → the host RESETS Big Picture to its HOME screen
//      (fires `steam://open/bigpicture`; no game pre-selected).
//   2. If THIS client is NOT already viewing a stream (`shellState !== "streaming"`)
//      → start/resume one stream to the primary target (targets[0]) — Moonlight
//      resumes a resumable host, so this both opens a fresh stream and reconnects to
//      one already live elsewhere.
//   3. If this client IS already streaming → the host-side BPM-home reset alone moved
//      the live session; don't start a 2nd local Moonlight process.
// Mirrors launchSteamGame() exactly (same one-session guard / stream path) but sends
// the bare `steam-bigpicture` instead of `steam-launch <appid>`. The gate is THIS
// client's own shellState, NOT the host's session flag.
function launchSteamBigPicture(root, steamBigPictureReq) {
    root.userActivity();
    // Fire the host-side BPM-home reset (fire-and-forget; reply is just ok/error).
    steamBigPictureReq.request("steam-bigpicture");
    if (root.shellState === "streaming") {
        // This client is already in the stream — the reset moved the live BPM.
        // Don't start a 2nd local Moonlight process.
        return;
    }
    // Not viewing a stream here → start/resume one to the primary target. With no
    // target configured there's nothing to stream into; the reset still fired.
    let ts = root.targets || [];
    if (ts.length > 0)
        root.streamRequested(ts[0]);
    else
        console.log("HomeScreen: steam-bigpicture sent, but no stream target configured");
}

// === Quit running Steam game choreography ===
// The active-game popover's "Quit" action →
//   1. `steam-quit <appid>` → the host gracefully terminates the running game
//      (SIGTERM to its process group — like Steam's Stop button). Fire-and-forget;
//      the reply is a status JSON (logged only on non-ok).
//   2. Close THIS client's Moonlight stream via the existing
//      streamQuitRequested(targets[0]) path (same teardown the Moonlight "Quit
//      Stream" action uses).
// The two race exactly like launchSteamGame's navigate+stream — the host kill and the
// local stream-close are independent.
function quitSteamGame(root, steamQuitReq, appid) {
    root.userActivity();
    // Fire the host-side graceful kill (fire-and-forget; reply is a status JSON).
    steamQuitReq.request("steam-quit", appid);
    // Close the local Moonlight stream to the primary target, if one is configured.
    let ts = root.targets || [];
    if (ts.length > 0)
        root.streamQuitRequested(ts[0]);
    else
        console.log("HomeScreen: steam-quit " + appid + " sent, but no stream target configured");
}
