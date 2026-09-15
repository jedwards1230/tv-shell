# tv-shell v2 — design

> **Status:** draft · 2026-09-05 · records the decisions taken on 2026-09-04 and 2026-09-05 and does not reopen them. It supersedes `docs/PRD.md` §5 "Compositor" and "Input arbitration" and §12 decision 5; every other PRD section stands. Open work is §13; the dated decision log is §14.
>
> Evidence base: the research reports listed in §15 and the live gamescope measurements of 2026-09-05 (jedwards1230/tv-shell#453, #454). Hosts are named by role only: the HTPC, the streaming host, the AVR, the TV. Two reviews (adversarial and architectural) were run over the first draft; their findings are folded in or listed in §13.

## 1. Goals and non-goals

**Goals**

1. The kiosk invariant — exactly one app fills the screen, backgrounded apps keep running and never share it — is enforced by a compositor primitive that no window can refuse and that cannot half-succeed.
2. Every claim the shell makes about the screen is verified against the compositor's own published state, and a claim that cannot be verified is a failure, never a silence.
3. 4K120 HDR10 with VRR on the TV, with a real 4K120 HDR Moonlight stream presenting at the refresh rate alone and with the shell drawn over it.
4. We do not write a compositor. As SteamOS does, we own what sits on top of gamescope: the shell, the pad daemon, AV control and the supervisor. One Rust core process owns policy; the shell UI is an ordinary client of the compositor with no privileged surface type.
5. In-repo supervision: a session target with a frame heartbeat, restart with backoff, and a couch-reachable and LAN-reachable escape hatch in every state.
6. The whole window model is testable in CI against a real headless compositor, and asserted in the field by the running core.
7. Moonlight and Steam Remote Play are both first-class, permanently supported, independently testable streaming clients (§12).
8. v1 keeps working on the couch until v2 replaces it; both are selectable sessions on the same install and neither can break the other.
9. CEC is the AV control authority, in its own daemon; the IP leg complements it on the cold path and recovers it on the warm path. The box never claims or releases the display unless it owns it. (Amended 2026-09-14 by §13 Q7, which reversed the original "IP is the AV authority; CEC is an observer".)

**Non-goals** (PRD §4 stands; these are the v2 additions)

| Not doing | Why |
|---|---|
| Layer-shell, tiling, or native-Wayland windows on the focus/HDR path | gamescope's focus control is X11-atom driven; its WSI layer exposes HDR only to X11 surfaces (§6) |
| Smithay, wlroots, or a Hyprland plugin as the compositor | No upstream HDR (Smithay, cage); ABI churn and the screencopy blind spot (Hyprland plugin). See `gamescope-eval.md` |
| A virtual-pad twin while a game is on screen | Games get the real evdev node (§7) |
| libcec / `cec-rs` in the core | Six contenders for one exclusive tty; the in-process design is the defect generator (§8) |
| Deciding the plugin mechanism now | Requirements only (§13 Q2) |
| SteamOS itself, an immutable root, nested per-app gamescope | Rejected 2026-08-29; unchanged. The Steam-as-shell fallback in §12 is a gamescope-session on our own OS, not this |

## 2. The vision that does not change

A 10-foot couch console: controller-first, B always goes back, every element D-pad reachable, sized for a 4K OLED at couch distance, HDR10 and VRR at 120 Hz. It launches Moonlight streams, local apps and web apps full-screen, owns the gamepad fleet exclusively, wakes and releases the AV chain on its own, and is machine-drivable over socket, HTTP, MCP and MQTT through one closed command vocabulary. The kiosk invariant is the product. PRD §1–§4 and §6.8 describe the finished couch UI and are not restated.

## 3. Why v1 failed

v1 re-fixed the kiosk invariant nine times in four months on Hyprland, and the symptom is still present after the ninth fix (jedwards1230/tv-shell#455, open).

| # | Date | PR | What it changed | Why it was insufficient |
|---|---|---|---|---|
| 1 | 05-27 | #49 | Window matching + resume | Matching only; nothing enforced fullscreen |
| 2 | 06-07 | #196 | Hyprland Lua window rules | Rule syntax churn; seven follow-up passes |
| 3 | 07-01 | #293 | Kiosk fullscreen on open | Open-only; a later window still took the screen |
| 4 | 07-01 | #294 | Continuous enforcement | Two racing enforcers on one workspace |
| 5 | 07-04 | #307 | Daemon self-heal + `KIOSK_WINDOW_MODEL.md` | Promised "one workspace per app", never built |
| 6 | 07-04 | #308 | Declarative lockdown, single enforcer | "Kills the root cause"; falsified in 15 days by #347 |
| 7 | 07-20 | #348 | Assert fullscreen after focus lands | Focusing a tiled window under a fullscreen one changes focus, not the screen |
| 8 | 08-26 | #444 | One workspace per app class | Found the July doc's "single workspace" clause was enforced by no code; falsified in 3 days |
| 9 | 08-29 | #448 | Accept the bare socket2 address | `openwindow` parsing required a `0x` prefix the compositor never sends; park never ran once. Unit fixtures asserted the shape the code wanted |

Three failure classes recur, all at the compositor boundary (`config/hyprland.conf`, `daemon/src/hyprland.rs`, `AppLifecycleManager.qml`, `shell.qml`; 204 commits mention focus; `fix:` outnumbers `feat:` 1.66:1; `shell/` has a 53% rewrite ratio and has not converged):

- **Silent success.** `hyprctl dispatch` exits 0 when its selector matches nothing. `ok` was returned for a dropped launch (#376), an escape that could not leave fullscreen (#436), an unparked window (#448), a stopped heartbeat (#402), and a compositor wedged for nine days (#383). The 2026-09-04 postmortem found zero `parked` lines across a whole boot on a binary containing #448; the residual cause (event listener attached, nothing processed) is still unknown, so v2 must not depend on an event path alone (§10).
- **The compositor answers about a different object.** As a layer-shell surface the shell never appeared in `activewindow`, which named a backgrounded toplevel instead (fixed by declaring ownership, #352). A focus request is something a window can decline: Remote Play's `streaming_client` reported `acceptsInput: false` and a live game became unreachable.
- **Exclusive-device contention.** The CEC adapter, the pads and the GPU each had more than one owner; each time the arbitration mechanism became the failure.

The two durable v1 fixes (#352 "declare ownership, never infer it"; #444 "a switch is a compositor operation no window can refuse") both removed a layer at which success could be reported without being true. v2 applies that to the whole boundary: gamescope's base-layer policy is unrefusable, it publishes the result as root-window atoms, and the core reads them back before it believes anything. The new risk is the mirror image: a name or shape the core writes that the compositor never reads (the scope name in §5 is the worked example). Every such contract is pinned in CI against the compositor's own bytes.

## 4. Architecture

Compositor: **gamescope**, DRM backend, `--steam` (SteamControlled focus policy), pinned 3.16.28 or newer (3.16.23 lacks the August keyboard-focus reclaim fixes and `focus_info`). Decided 2026-09-05 on the measured numbers in §6. Nothing below is a compositor of our own: the unit shape is SteamOS's, and the ChimeraOS `gamescope-session` unit files and scripts (ready-fd handshake, environment dump, short-session tracker, session select) are the starting point for the supervisor rather than units written from scratch.

```
display manager (autologin, Relogin=true) ── selects one of:
  tv-shell-wayland.desktop   → v1 session (Hyprland + Quickshell + tv-shell-input)   [unchanged]
  tv-shell-v2.desktop        → v2 session script: stop any stale target, reset-failed,
      mkfifo ready + stats, systemctl --user start --wait tv-shell-session.target
        ├─ tv-shell-gamescope.service   Type=notify, NotifyAccess=all, TimeoutStartSec=5,
        │     Before=graphical-session.target; -R <ready fifo> → env dumped to
        │     %t/tv-shell-gamescope-environment, then systemd-notify --ready
        │     ├─ Xwayland :0  ← shell, overlay/QAM, notifications (X11 clients, self-tagged)
        │     └─ Xwayland :1… ← one server per launched app (GAMESCOPE_CREATE_XWAYLAND_SERVER)
        ├─ tv-shell-core.service        Upholds=, After=graphical-session.target, EnvironmentFile=-%t/…
        ├─ tv-shell-shell.service       Upholds=, After=…  — the UI, an ordinary X11 client
        ├─ tv-shell-stats.service       Wants=            — sole reader of the stats FIFO (§9)
        ├─ tv-shell-panel.service       Wants=            — LAN recovery + observability (survives core death)
        └─ tv-shell-v2-cec.service      Wants=            — kernel-CEC AV control (ConditionPathExists=/dev/cec0)
      apps: app-steam-app<id>-<pid>.scope, one per launch, on their own Xwayland server
```

**The v2 session file is `tv-shell-v2.desktop`, not the `tv-shell-gamescope.desktop` this section first named.** That name was already taken by the Ansible-owned gamescope measurement prototype (`dev/gamescope/README.md`), which is §10's regression bench and still gets selected — so reusing it would have overwritten a live session entry. `scripts/install-v2.sh` writes the v2 one, and a test asserts the name collides with neither it nor v1's `tv-shell-wayland.desktop`.

`BindsTo`/`Upholds`/`Wants` carry no ordering; ordering is `graphical-session.target`, which becomes active only after gamescope's `READY=1`, and that is sent only after the environment file exists. The session script's `--wait` is what makes "gamescope dies → the session exits" true.

| Component | Owns | Never does |
|---|---|---|
| gamescope | Output mode, HDR/VRR state, which app id is on screen, keyboard/mouse focus, overlay plane, screenshots | Touch gamepads; know apps beyond an id |
| core (`tv-shell-core`, the daemon's successor) | Base-layer list; app launch, scope, tag; pad grab and presenters; the command vocabulary and the event stream; IPC, HTTP, MCP, MQTT, metrics; HDR toggle; field assertions | Draw anything; own AV control (§13 Q7 gave that to its own daemon — a backend that can hang must not sit in the process that keeps Moonlight alive); hold a privileged surface; keep state on disk that the X server and systemd already hold |
| shell (X11 client, fixed app id) | Home, Library, Settings, Widgets; drawers and QAM as separate `STEAM_OVERLAY` toplevels; tags its own windows | Talk to hardware; ask "who is on screen" |
| per-player presenters (uinput) | One clean virtual pad per player, always present; only grab and routing toggle | Appear and vanish (that is a hotplug to every game) |
| stats relay | Tail gamescope's stats FIFO into a file/socket both core and panel read | |
| panel | Unit restart, journal, deploy, supervisor state, escape hatches | Hold another node's bridge token |
| CEC daemon (`tv-shell-cec`) | The AV control backend: power, input switching and volume over the kernel CEC API; the display-ownership sensor; the IP leg (AVR telnet, TV WoL) and the warm-path failover between them | Share the adapter; claim or release the display without positive proof; broadcast a `<Standby>` |
| `tv-shell-host` (streaming host) | Steam library, launch, quit, sleep | Unchanged from v1 |

**Carried over unchanged in contract:** the Unix-socket IPC framing and reply grammar — newline-framed requests, and `ok` · `unknown` · `error:<msg>` · a bare compact JSON document in reply. That is the wire shape, not the verb list: the `intent` vocabulary is **not** carried over (§7 splits it by owner, and the event stream added there rides this same framing). Also unchanged: the four-transport control surface over one `bridge_core`, the settings store with one writer, the capability handshake, MQTT/HA topics, the `/metrics` catalogue, web apps, the Steam sidecar, `shell-state` (media state for the HA suspend rule, a separate channel from the frame heartbeat), and logind `Active` watching (grabs release when another session takes the seat).

**Changed or deleted** (the IPC table in `docs/IPC_PROTOCOL.md` gets a deleted/renamed/kept column when the core lands):

| Item | Fate |
|---|---|
| `daemon/src/workspaces.rs`, `hyprland.rs`, `session_env.rs`, the park/reconcile paths, `hypr-*` commands and `hypr:*` events | Deleted; `screen-state` (a read of the gamescope atoms) replaces `hypr-active`/`hypr-clients`/`hypr-monitors` |
| `display_mode.rs` (daemon and panel): `monitor=` rewriting with a revert timer | Mode is pinned in `[display]`; a resolution/refresh change is a gamescope restart under the same confirm-or-revert contract; VRR and HDR are root-atom writes |
| `shell-focus on|off`, `overlay-focus` | The compositor publishes ownership; the shell's declaration survives only as the grab trigger (§7) |
| `grab`/`release`/`handoff`, `[input.contracts]` | Presenters collapse to two (§7) |
| The `intent` vocabulary, the `intent:*` broadcast, `key up\|down\|left\|right\|select\|back` | Retired, not ported: split by owner into core operations, a shell-local surface and real input injection (§7). The keyboard escapes it carried become `gamescope-action-binding` entries |
| `resumeFocus.js`, `prewarm.js`/`appQuirks.js` launch paths, `HyprctlClients.qml`, every fullscreen assertion, the layer-shell `PanelWindow`, the QML duplicates of display-mode and CEC-wake | Deleted or rewritten against the core |
| `WorkspaceAudioMuter.qml` / `audioOwnership.js` | Re-keyed to the base window's app id; the pure attribution logic is kept |
| `config/hyprland.conf*`, `scripts/super-intent.sh`, `tv-shell-session.sh` | Stay, v1-only |
| Suspend | Already one `power-suspend` path; v2 adds the policy gate (display ownership + media) it lacks |
| `settings_consumer_table()` | Every key whose reader is deleted (`hdrEnabled`, `nightLight*`, `overscan`, the `cec*` focus keys, `prewarmApps`, `wakeOnController`) names its new consumer or is dropped in the same PR |

## 5. Focus and window model

The invariant becomes a data structure gamescope owns. Under `--steam`, every mapped X window resolves to an app id and the base layer is the first id in an ordered list.

| Atom (X root unless noted) | Writer | Meaning |
|---|---|---|
| cgroup scope `app-steam-app<id>-<launcherpid>.scope` | core, at launch (`systemd-run --user --scope`) | **Primary id.** gamescope's only cgroup parser is `sscanf("app-steam-app%u-%d.scope")` (`src/Utils/Process.cpp`), evaluated at window creation from the XRes client pid. The prefix is an upstream contract, not ours to rename; it resolves before any core action and survives a core restart |
| `STEAM_GAME` (per window) | core (repair) / shell (its own windows) | Override for a window whose scope did not resolve (pid namespace, a browser that handed off to a running instance); authoritative when present |
| `GAMESCOPECTRL_BASELAYER_APPID` | core | Ordered list; first id with a mapped window is on screen. "Show app" = `[<id>, <shell>]`; "home" = `[<shell>]`; an app exiting falls back to the shell with no compositor action |
| `GAMESCOPE_FOCUSED_WINDOW`, `GAMESCOPE_FOCUSED_APP`, `GAMESCOPE_FOCUSABLE_WINDOWS`/`_APPS` | gamescope | Published result. **The base window is `GAMESCOPE_FOCUSED_WINDOW`**; `GAMESCOPE_FOCUSED_APP` reads empty while an input-focus overlay is up (measured), so every rule below keys on the window's app id |
| `GAMESCOPE_CREATE_XWAYLAND_SERVER` / `_FEEDBACK` (`"<identifier> <server_id> <display>"`), `GAMESCOPE_DESTROY_XWAYLAND_SERVER`, `GAMESCOPE_XWAYLAND_SERVER_ID` (per server root) | core / gamescope | One Xwayland server per app; `GAMESCOPE_FOCUS_DISPLAY` says which server holds the keyboard |
| `STEAM_OVERLAY=1` + `STEAM_INPUT_FOCUS=1`, `STEAM_NOTIFICATION=1` (per window) | shell, on its own toplevels before map | Drawer/QAM over a running app takes keyboard and mouse without changing the base layer (measured at 120 fps over an SDR base); toasts |
| `STEAM_STREAMING_CLIENT` (per window) | Steam | Remote Play client; always a focus candidate under SteamControlled (§13 Q3) |

Rules the core enforces:

- **Scope first, tag as repair, never by name.** Moonlight's stream window is a second X window replacing its main one (kit defect K6); a name-keyed tag lands on a window about to die. The core derives pids with `XResQueryClientIds` as gamescope does (not the client-asserted `_NET_WM_PID`), watches `MapNotify` on every server, and writes `STEAM_GAME` only where the scope did not resolve.
- **Multi-process apps get a class-keyed fallback.** A second Chromium `--app` launch hands off to the running browser and exits, so neither scope nor pid names it; each web app runs with its own `--user-data-dir`/`--class`, and the launch table matches on pid or `StartupWMClass` (§13 Q4).
- **Shell windows self-tag.** The shell is a service, not a launched app, and its drawers share its pid, so the shell process sets `STEAM_GAME` on its base window and `STEAM_OVERLAY`/`STEAM_INPUT_FOCUS`/`STEAM_NOTIFICATION` on each overlay toplevel before mapping it. That needs `XChangeProperty` access from the shell runtime (§13 Q1) and makes drawer, QAM and toasts separate X toplevels.
- **One write, then verify.** A switch is one `GAMESCOPECTRL_BASELAYER_APPID` write followed by a read of `GAMESCOPE_FOCUSED_WINDOW`'s app id within a bounded window (measured 14–19 ms). A mismatch is an IPC error, a metric and a log line, never `ok`.
- **Transient unmaps are held, not followed.** An app whose last window unmaps for a moment (Moonlight main → stream window, a browser navigation) would drop the base layer to the shell, flip grab and audio, and flip back. The core pins `GAMESCOPECTRL_BASELAYER_WINDOW` across known transitions and applies a short hysteresis before treating a fallback as an exit. Exit fallback itself is unmeasured and is a bench row (§10).
- **Audio follows the base window.** "You hear the workspace on screen" becomes "you hear the base window's app id"; PipeWire nodes are still attributed by `application.process.binary`.
- **The shell's app id is private** (the kit used 9001). Under `--steam`, 769 is the Steam client's own id (`window_is_steam`: forced fullscreen sizing, `focus=steam` in the stats pipe) and is reserved for the Steam client when it runs as an app (§12); the shell sizes itself to the output instead of inheriting that path.

## 6. Display and HDR

Measured 2026-09-05 on gamescope 3.16.23, an AMD iGPU, through the AVR to the TV; the kit is `dev/gamescope/` and is the regression bench (§10). Every number below was taken with `--hdr-enabled` on the command line, a single Xwayland server, and an SDR prototype shell as the base layer; the cutover re-measures on the pinned build through the runtime atoms and the per-app-server topology.

| Criterion | Result | Method |
|---|---|---|
| 3840x2160 @ 120 Hz | PASS | `active CRTC mode: 3840x2160 120.00`; `GAMESCOPE_DISPLAY_REFRESH_RATE_FEEDBACK=120` |
| HDR10 signalling | PASS | `Colorspace = BT2020_RGB`, `HDR_OUTPUT_METADATA` blob with EOTF = PQ |
| VRR | PASS | `VRR_ENABLED=1`, `VRR Active: true`, `GAMESCOPE_VRR_FEEDBACK=1` |
| 4K120 HDR Moonlight stream alone / with shell switched over it / back | PASS | stats FIFO `fps=120.000000` throughout, never 60; HEVC Main10, VAAPI on X11 |
| HDR10 swapchain exposed to an X11 client | PASS | WSI: `server hdr output enabled: true`, `hdr formats exposed to client: true`; swapchain `A2B10G10R10_UNORM_PACK32 / HDR10_ST2084_EXT`; no forced bypass needed |
| Focus switch | PASS | 20 of 20, 14–19 ms |
| Overlay over a running app | PASS over an SDR base | app stays base window at 120 fps, focus returns on close. **Not yet measured over the HDR stream** |
| Output bit depth | UNMEASURABLE | debugfs prints `Maximum: 12` with no `Current:` on this kernel; `max bpc` is a requested cap (gamescope leaves 16). The TV's info panel is the reading of record |
| Per-app Xwayland servers, app-exit fallback, static shell under VRR | NOT EXERCISED | bench rows in §10 |
| SDR black floor, pad reaches shell, Qt usable | eyes-only, pending | |

Design rules that follow:

- **Mode is pinned in config** (`[display]` width/height/refresh → `-W -H -r`): the EDID preferred mode is 60 Hz.
- **HDR is a runtime switch, SteamOS-style.** The core sets `GAMESCOPE_DISPLAY_HDR_ENABLED` on the root and reads `GAMESCOPE_HDR_OUTPUT_FEEDBACK` (= EDID HDR10 && `hdr_enabled`) and `GAMESCOPE_DISPLAY_SUPPORTS_HDR`. **Never a bare `gamescopectl <convar>`**: on 3.16.x and master a value-less call resets the convar to its default and turns HDR off with no log line (the phase-2 "feedback 0" was self-inflicted this way).
- **The hotplug window.** An HDMI re-negotiation (~1 s) zeroes `GAMESCOPE_HDR_OUTPUT_FEEDBACK`, `GAMESCOPE_DISPLAY_SUPPORTS_HDR` and `GAMESCOPE_VRR_FEEDBACK`, then restores them; a Vulkan surface created inside it stays SDR for its life. The one observed instance coincided with the Moonlight launch to within a second, and its cause is open (the AVR, an audio-format renegotiation on stream start, the v1 CEC lifecycle; §13 Q9). Policy: gate an HDR-capable launch on the feedback atom reading 1 for a settle period; detect an SDR-stuck client from its own HDR feedback, not by scraping its log; the relaunch-once fallback is provisional until the cause is known, because if the launch causes the hotplug it would kill every first stream.
- **HDR clients are X11 clients** under the WSI layer (`ENABLE_GAMESCOPE_WSI=1`, `QT_QPA_PLATFORM=xcb`). The layer hard-codes `hdrOutput = false` for native-Wayland surfaces in every version and gamescope serves no colour-management protocol. The window must match its toplevel within 2 px, else `GAMESCOPE_WSI_FORCE_BYPASS=1`. Moonlight 6.1.0 on gamescope's native Wayland segfaults anyway (gamescope#2261 family).
- **`CONFIG_AMD_PRIVATE_COLOR` is optional.** It gates only the AMD plane colour pipeline (direct scanout with hardware TF/LUTs). Without it gamescope composites in Vulkan, and that cost was invisible at 4K120 on this GPU for a base-layer switch; the overlay-over-HDR composite is the unmeasured case.
- **SDR in HDR.** `--hdr-sdr-content-nits` sets the shell's white; the black floor is criterion 4 (eyes). gamescope#1887 (SDR oversaturated on an HDR output) is the known risk for the shell's own colours.
- **VRR default is an open question** (§13 Q11): the ops record has OLED near-black flicker and AVR OSD notes recommending VRR off.

## 7. Input

This supersedes PRD §5 "Input arbitration" (four presenters).

> **The plan for actually building this — phases, open decisions, and the hardware
> measurements it is gated on — is [`V2_GAMEPAD_HANDOFF.md`](V2_GAMEPAD_HANDOFF.md).**
> Two questions that document carried as open are now closed by measurement on htpc-1,
> and both are load-bearing here. **The core can drive the shell with a uinput keyboard**
> (M0, 2026-09-10): a synthesised key arrives exactly as a physical one does, with
> Moonlight running and suspended, and needs no seat tagging. The 2026-09-09 negative was
> an artefact of the probe key — gamescope silently drops `KEY_MENU`, which was the
> drawer's only binding (jedwards1230/tv-shell#489, fixed in #490 by moving the drawer to
> Tab), so any code the core emits must be verified to arrive rather than assumed. And
> **an idle presenter is enumerated by a running app** (M1, 2026-09-10): with the physical
> pad ungrabbed, Moonlight opened the virtual node within ~2 s and held it alongside the
> real one, so the app saw two controllers. That settles the grab question against the
> `gamepad` contract as originally written; see the amended bullet below.

- **The core keeps `EVIOCGRAB` of the pad fleet**: DB-match-or-reject discovery, stable per-player slots, hot join/leave, rumble/battery/LED, per-player uinput presenters. gamescope never opens joystick nodes (libinput ignores the class; true in SteamOS too), but a pad's companion touchpad/motion nodes present as pointers gamescope will read; discovery claims or inhibits them (SteamOS ships `ds-inhibit` for this).
- **Two contracts, not four.** `gamepad` (the default, games and streams): the grab is held and the app reads the per-player presenter, so exactly one device ever moves. `keyboard` (web apps, Plex): the grab stays and the core translates the pad to a uinput keyboard, since a browser reads no gamepad. `handoff` collapses into `gamepad`. **This bullet is a reversal, recorded rather than rewritten.** It said until 2026-09-10 that the physical node is *ungrabbed* while the app is the base window, so the game sees the real pad and no virtual twin double-fires. M1 disproved the second half — the idle presenter is enumerated anyway, so the app sees two pads — and the first half makes held-button masking impossible, since the press reaches the real device and nothing can swallow it: jedwards1230/tv-shell#295 reintroduced by construction. Grab-always is option A in [`V2_GAMEPAD_HANDOFF.md`](V2_GAMEPAD_HANDOFF.md) §4 Q1. It costs a hop of latency, and rumble/battery/LED must eventually be proxied back through the core — named there as a follow-up, deliberately not built on the handoff path.
- **Routing follows visibility, devices do not.** Under grab-always the grab itself no longer moves: what changes on a visibility edge is where the grabbed stream is *routed*. The route points at the shell key-map when the shell is the base window or a `STEAM_INPUT_FOCUS` overlay is mapped, and at the app's presenter otherwise; the uinput presenters stay present throughout (create/destroy is a hotplug event every game and Moonlight forward to the streaming host, #402). Sequence: overlay maps → quiesce the app's presenter → mask held buttons → route to the shell key-map; overlay unmaps → quiesce the shell keyboard → unmask → route back. A Guide tap still reaches the app before the hold threshold, since Guide is buffered and released on a tap; accepted, as in v1's Handoff.
- **Escapes.** The Meta hold and the safety combos come from a passive, non-grabbing reader in the core, so they are unrefusable by the compositor but depend on the core being alive (`Upholds=` is the mitigation). The keyboard escapes (`Super`, `Super+Escape`, `Super+Backspace`) are `gamescope-action-binding` entries and survive a dead core.
- **Keyboard stays with the compositor.** gamescope routes keyboard focus to the base window or the overlay deterministically (`GAMESCOPE_FOCUS_DISPLAY`); the shell reads keys through Qt, and automation injects nav keys via a uinput keyboard (or libei through `gamescope-eis`).

**The `intent` surface is retired, not ported.** It bundled three unrelated concerns behind one flat string vocabulary. That was coherent while the daemon was the only backend and the shell was its client; it stops being coherent once the core owns focus and the shell is a peer app among others. v2 splits it by owner:

| Concern | v1 form | v2 form | Owner |
|---|---|---|---|
| Put an app on screen | `intent app:<wmClass>` | `show <appid>` / `home` — already in `core/` | core |
| Launch an app | the shell shelled out | `launch <appid>` against the `[[app]]` class table — already in `core/` | core |
| Observe the world | polling plus ad-hoc broadcasts | one event stream subscribers read | core → many |
| Navigate the UI | `intent settings:bluetooth`, `overlay:volume`, `menu` | the shell's own surface; the core does not proxy it | shell |
| Synthesise a keypress | `key up\|down\|left\|right\|select\|back` | real input injection (uinput, or libei through `gamescope-eis`) — not an IPC verb | core input layer |
| Escape a wedged UI | `intent home-hold`, a message to the shell | a passive core-side reader performing a base-layer write | core |
| See the screen | `GET /screenshot` / MCP `take_screenshot`, both `grim` | `screenshot <path>` — a `GAMESCOPECTRL_REQUEST_SCREENSHOT` write, already in `core/` | core |

The "see the screen" row is the one that had no v2 answer at all until it was measured. gamescope 3.16.28 implements **no** Wayland screen-capture protocol — neither `wlr-screencopy-unstable-v1` nor `ext-image-copy-capture-v1` is in its tree — so `grim` fails by construction and every v1 surface built on it (`GET /screenshot`, MCP `take_screenshot` and `screenshot://current`, the panel's Dev ▸ Screenshot page) is dead under v2. Reading pixels from Xwayland is dead too: it runs `-rootless` under manual Composite redirection, so the root is `BadMatch` and a GPU-rendered window reads back black — and with `--expose-wayland` the shell is a Wayland client with no X window anyway. What works is asking gamescope to composite and write the frame itself, which is the same shape as every other row here: the core expresses an intent through a root property and never inspects pixels. Two upstream properties of that path are traps and are handled as rules in `core/src/screenshot.rs`. The request property is **not** a completion signal: measured on hardware it cleared at 32 ms against a file that landed at 712 ms, and gamescope clears it on the write-failure path too — so the wait is on a complete file (signature + `IEND`; gamescope writes in place with no rename) and the property only records *when* it cleared. And the output path is one hardcoded shared file, so a failed capture would otherwise hand back the previous screenshot as a plausible-looking current one — hence clearing it before asking, which makes staleness unrepresentable rather than merely detectable.

The escape row is the one that gets strictly better, and it is why this is a split rather than a tidy-up. In v1 the escape was a message to the shell, so it failed precisely when the shell was wedged — the only condition under which anyone reaches for it. In v2 the core owns the base layer, so the combo performs a core operation directly and works with the shell dead. That is the property `intent` was reaching for and never had.

Five rules follow, and they outlive this decision:

1. **A closed vocabulary, enforced by the type system rather than by naming convention.** The genuinely good part of `intent` was that its vocabulary was closed and enumerable. v2 gets that from `Command` in `core/src/protocol.rs`, where an exhaustive match forces every consumer to account for a new operation at compile time. Extend that enum; never add a parallel stringly-typed channel beside it.
2. **Illegal states stay unrepresentable.** Already load-bearing in the crate — `Supervised` is a token that cannot be forged, `AppId` wraps a private field — and here it is the reason no `String` command name flows through the core.
3. **The event stream publishes state, not deltas.** A broadcast channel lags slow subscribers. If every event is a full snapshot, a lagged or reconnecting subscriber takes the latest one and is correct; with deltas, lag corrupts and reconnect needs replay. That is what makes adding the stream safe at all.
4. **One wire format.** Events ride §4's framing unchanged — newline-framed, `ok` · `unknown` · `error:<msg>` · a bare JSON document. No second protocol beside it.
5. **One writer per piece of state.** v1's rule that the daemon is the sole settings writer was right and survives verbatim; the alternative is read-modify-write races between the shell and everything else that can set a key.

## 8. AV control

The chain is HTPC HDMI → the AVR (video, plus a CEC-only leg to a USB CEC adapter) → the TV. Facts that fix the design: the AVR in standby powers down its CEC line and its NIC unless its menu enables network control in standby; the AVR's telnet port accepts one client; a TV cold-start needs WoL plus the TV's own wake-over-network setting; a receiver ignores CEC from non-selected inputs (it ACKs the frame and then does nothing, so a transmit that "succeeded" proves nothing); the adapter's tty has one owner and the loser fails silently.

**This table was rewritten on 2026-09-14 to match §13 Q7.** It had survived the Q7
reversal unamended, so the document said both things at once — and this is the section a
reader goes to for AV ownership.

| Concern | v2 owner | Mechanism |
|---|---|---|
| Wake / input select / standby of the AVR | `tv-shell-v2-cec`, **CEC primary** | `<Image View On>` + `<Active Source>` to claim, `<Set Stream Path>` to hand the display away, an **addressed** `<Standby>` to sleep. Volume and standby only work while this box is the AVR's selected input, so ordering is load-bearing: claim first, then act |
| The same, over IP | `tv-shell-v2-cec`, IP | Denon/Marantz ASCII telnet (`PWON`, `SI<input>`, `Z2OFF`, `PWSTANDBY`). `av_net.rs` from the closed, unmerged PR #191 **has been ported** — to `cec/src/ip/avr.rs`, on typed `cec.toml` rather than the original's env vars. It runs on the **cold path** and as **warm-path recovery**; see the capability note below |
| TV cold wake / power off | `tv-shell-v2-cec`, IP | **Wake-on-LAN only** (`cec/src/ip/wol.rs`). There is no LG webOS / SSAP client in this tree, none was written, and none should be: WoL is the only IP operation the television genuinely needs that CEC cannot do, and a webOS client is a second auth surface with a documented history of breaking across firmware — one of the three reasons Q7 demoted IP in the first place. Power *off* is CEC's addressed `<Standby>`, not an IP call |
| Display ownership | `tv-shell-v2-cec` | The passive gate is unchanged (`owns_display()` needs positive proof; `may_claim_active_source()` yields to a known other owner), and the witnesses are **reversed** from the 09-04 text: `<Active Source>` on the CEC bus is the **primary** sensor, and the AVR's `SI` push events are the **secondary, IP-path** witness |
| Theater sleep on idle | core | Only when the HTPC is the AVR's selected input; otherwise release nothing |
| CEC | `tv-shell-v2-cec`, kernel driver | **The primary AV control backend**, owning power, input switching and volume over the kernel CEC API (`/dev/cecN`, `pulse8-cec`). Sole owner of the adapter. libcec and `cec-rs` leave the tree with the static-link CI leg |
| TV remote passthrough | goal, not constraint | No evidence of routine remote use; it was silently dead for weeks in v1. Kernel `pulse8-cec` re-creates the evdev input device libcec replaced, so remote keys would arrive through evdev if they are ever wanted — no forwarder is ported |

**The IP leg is a capability complement on the cold path and a failover on the warm path,
not a fallback only.** Two operations have no CEC expression at all:

- **AVR Zone 2 is not CEC-addressable.** `Z2OFF` has no CEC equivalent, so where Zone 2
  matters the telnet session runs on *every* standby — with a perfectly healthy bus.
- **A fully-off television cannot be cold-woken by CEC.** `<Image View On>` reaches nothing
  at mains standby; that needs WoL — as does the AVR itself when its
  network-control-in-standby setting is off.

So the wake and standby sequences fire the IP steps *before* the CEC steps unconditionally,
and `cec/src/failover.rs` decides only the **warm-path** authority. Reading §8 as "IP is
what you use when CEC is broken" understates it by exactly those two capabilities.

Site preconditions (deployment, not code; listed in §11 cutover): AVR network control in
standby enabled and no other telnet client holding the port (it accepts one); TV
wake-over-network on. **No webOS pairing key is needed** — that precondition goes with the
webOS client that was never written. The v1 modprobe blacklist of the kernel CEC driver must
be reversed — now `htpc_cec_owner` in `jedwards1230/homelab-ansible#338`, derived from the
selected boot session so choosing v1 again puts the blacklist back — and the Plex `bwrap`
hiding of `/dev/ttyACM0` stays (verified still in place 2026-09-14; `inputattach` holds the
tty under the kernel path, but it has **not** been confirmed to take `TIOCEXCL`, so a second
opener may still corrupt the serio stream).

## 9. Supervision and recovery

This reverses PRD §12 decision 5: sensor and actuator both live in the repo. The sensor has to earn that: gamescope's stats FIFO emits one `fps=` line per 300 paints, so on a static base layer under VRR it is silent for as long as nothing draws. The heartbeat is therefore a **forced-paint probe**: once a second the core damages a 1 px core-owned window (or issues a debug repaint), then waits a bounded time for the next `fps=`/`focus=` line. The FIFO has one reader, gamescope never reopens it, and lines written with no reader are dropped, so a small relay unit tails it into a file both core and panel read and holds the fd across core restarts. Its false-positive rate is a cutover number (§11).

| Failure | Sensor | Response |
|---|---|---|
| gamescope dies | `BindsTo=` stops the session target | Session script's `--wait` returns; autologin restarts the session |
| Shell dies | `Upholds=` on the shell unit | Restart under the live compositor; the base-layer list re-resolves to the shell |
| Shell crash-loops | Short-session tracker (`ExecStartPre`/`ExecStopPost`; 3 exits under 60 s) | Backoff, then a deployment hook selects the v1 session (a root-owned display-manager file; §13 Q8) |
| Core dies | `Upholds=` | Restart. On start the core is stateless: it reads `GAMESCOPECTRL_BASELAYER_APPID` back as its last intent, enumerates servers by `GAMESCOPE_XWAYLAND_SERVER_ID`, rebuilds the launch table from `app-steam-app*.scope` units, and never writes "home" on boot (that would yank a live game) |
| Frames stop | Forced-paint heartbeat, `tv_shell_frames_presented_total` | Stall over N s with a mapped base window → restart the shell; persisting → restart the session target. Session restart stays manual until the false-positive rate is measured |
| Compositor wedged but alive | Same relay file, read by the panel | The panel, independent of the core, restarts units and shows supervisor state |
| Stuck in an app | Meta hold → `intent home` = one base-layer write | Unrefusable by the compositor; the "only recovery was `kill -KILL` from another machine" class (#436) closes |

Three requirements on the supervisor come out of the 2026-09-05 Steam measurements and the v1 ops record:

- **Invisible prompts are a stall class of their own.** The WSI-layer failure above asked its question through a `zenity` dialog. Under gamescope that is an untagged window nobody can see, and the client blocked for eight minutes at 0% CPU behind it — a hang with no log line and no visible cause. The supervisor must **tag or auto-dismiss unknown windows from a known process family** (so an unexpected dialog becomes visible or disappears rather than blocking), and set `GAMESCOPE_ZENITY_DISABLE=1` in the launch environment so a layer failure exits loudly instead of hanging black.
- **Only one supervisor may hold restart authority on the box.** The `htpc_common` Ansible role ships a **CEC watchdog** as a *system* unit that probes `cec-health`, reads a deliberately stopped `tv-shell-input` as a wedged adapter, and restarts it — re-grabbing the pad and silently undoing input fixes (jedwards1230/homelab-ansible#266). v2 introduces its own supervisor with restart authority, so at cutover the two would contend on one box.

  **The stand-down is DONE.** `htpc_cec_watchdog_active` derives from `htpc_boot_session`, htpc-1's boot session is already `tv-shell-v2`, and the timer was read on the box on 2026-09-14 (read-only) as `disabled` / `inactive`. Nothing further is owed on the tv-shell side. Two things are still true and must stay said: the unit and timer **still exist on disk** under Ansible's ownership, so this is a disable rather than a removal and they must never be re-enabled; and the probe they run reads v1's `cec-health` IPC verb, which does not exist in v2 at all.

  **What actually RETIRES it** — rather than merely disabling it — is the `Type=notify` + `WatchdogSec=` design of `tv-shell-v2-cec.service`. systemd becomes the only supervisor on the box with restart authority over the AV daemon, on a bounded timer, with no polling script and no side effects on a shared bus; and the `WATCHDOG=1` feed is gated on exactly one *observed* fact (`CEC_ADAP_G_CAPS` round-tripping on our own fd), not on a published verdict. That is the structural answer to the old failure, in which health was inferred from the outcome of our own transmits and then inferred a second time from IPC reachability — which is how a deliberately stopped daemon read as a wedged adapter and got "recovered" three times.
- **Sunshine and Steam Remote Play must not target the same streaming host at once.** When both did, Sunshine held the host running its own "Steam Big Picture" app and Remote Play faithfully captured *that*, so the host's Big Picture appeared instead of the game — for an hour, looking exactly like a capture bug. The kit already refuses a Moonlight stream against a busy host and names the running app; nothing does so on the Steam side. The supervisor owns this **interlock** — check the host's session before starting either client, and refuse with the running app named — rather than leaving it to whoever is on the couch.

**Steam owns `GAMESCOPECTRL_BASELAYER_APPID` while it runs, and the core must be designed for that, not against it.** The measurement above is not drift the core can win: Steam rewrote the atom on every stream start and stop and dropped our id from `GAMESCOPE_FOCUSABLE_APPS`. The core therefore **reconciles after Steam's writes** — reading the list back as its last intent (as the table's "Core dies" row already requires) and re-asserting only when it has an intent of its own to express — and never expects to hold the atom while Steam runs. Steam is an active adversary on that atom, not a source of drift.

The panel becomes the recovery and observability surface for the supervisor (unit states, heartbeat, last assertion failures, base-layer list, Xwayland servers, escape hatches), with its recovery tier still independent of the core; its unit names become config so the v1 and v2 panels can coexist.

## 10. Verification

**CI, headless.** `gamescope --backend headless` in a container with a software Vulkan device (lavapipe), Xwayland, and the pinned gamescope built from tag; no seat, no `/dev/uinput`, so the core runs with input disabled. Scripted X11 clients (`xprop`, the kit's `focus.sh`/`launch.sh`) exercise the whole contract: scope resolution (an untagged client in an `app-steam-app*` scope must appear in `GAMESCOPE_FOCUSABLE_WINDOWS` with that id), tag repair, base-layer ordering, app-exit fallback and the hysteresis, overlay focus, per-app Xwayland create/destroy, and the core's atom round-trips. Fixtures are real compositor bytes, not hand-written shapes (the #448 lesson). Whether headless emits stats lines and honours `GAMESCOPE_CREATE_XWAYLAND_SERVER` is unverified (§13 Q10); until the job passes, verification is the live bench only and the doc says so.

**Field assertions** in the running core, **polled, not event-driven** (the v1 residual defect was an attached listener that processed nothing), exported as metrics and on `GET /status`, alerting on any non-zero:

| Assertion | Reads |
|---|---|
| No untagged core-launched toplevel | launch table vs `GAMESCOPE_FOCUSABLE_WINDOWS`, filtered as gamescope filters (override-redirect, 1x1 and skip-taskbar windows excluded) |
| Base window equals intent | `GAMESCOPE_FOCUSED_WINDOW`'s app id == first mapped id of the list the core last wrote |
| Map events seen vs windows seen | a dead listener is visible as a widening gap |
| Frame heartbeat advancing | forced-paint probe |
| HDR atom expected | `GAMESCOPE_HDR_OUTPUT_FEEDBACK` == configured, outside the hotplug settle |
| Grab state matches visibility | grab armed ⇔ shell or overlay is the focus window |
| Exactly one shell, one core | unit MainPIDs |

**The measurement kit is the regression bench.** `dev/gamescope/measure.sh`, `focus.sh`, `launch.sh` and the stats reader stay in the tree as the live 4K120 HDR check after every gamescope or kernel bump, with the §6 pass table plus new rows: `STEAM_OVERLAY` over a 4K120 HDR stream (`fps=` and Moonlight's dropped-frame counter), a static home screen under VRR for five minutes (heartbeat lines per minute), app-exit fallback timing (observed working under the prototype 2026-09-05: closing Moonlight returned cleanly to the prototype shell), a window on a secondary Xwayland server getting the HDR bypass, and keys routed across servers. Every deployable artifact reports a real version (shell and core included); `tv_shell_build_info` reads git live and is not a restart signal.

## 11. Deploy and migration

- **The installer is `scripts/install-v2.sh`**, a separate script rather than a mode on v1's `scripts/install.sh` — the rule below applies to the installer itself, and a flag whose default is v1 is one forgotten argument away from installing over the running appliance. It defaults to the `/opt/tv-shell-v2` prefix and refuses a `--prefix` at or under `/opt/tv-shell` — normalised with `realpath -m` first, because an exact string compare let `/opt//tv-shell` through, and that is the one failure on this path that is silent and destructive rather than loud. It writes `tv-shell-v2.desktop` so a standalone install is selectable immediately, and takes `--no-session` to suppress that on a host where the deploy role owns the session entry — the split is one writer per file, with the role's copy winning on a managed host because only it can render the session env into `Exec=` as a `/usr/bin/env` prefix and only its toggle removes the entry. It installs the four v2 units with their `@TV_SHELL_V2_PREFIX@` token substituted — the fourth, `tv-shell-v2-cec.service`, joined at the 2026-09-14 CEC cutover and is `ConditionPathExists=/dev/cec0`-gated, so laying it on a box with no adapter is a recorded skip rather than a failure. Eight tests in `core/src/config.rs` run it into a scratch tree and assert the installed units carry no token and no path under v1's prefix, that every spelling of v1's prefix is refused while a normal one is accepted, and that `--no-session` suppresses exactly the session entry and nothing else. What it does **not** yet give the core is a release stream or an Ansible pin (both below).
- **Beside, not instead, at every shared layer.** v2 has its own session entry, install prefix and git clone (a `/dev/deploy` of a v2 branch must not replace v1's `shell/`), its own config file (the v1 daemon's `config.toml` root is `deny_unknown_fields`, so a new v2 table would abort v1 at startup), its own core binary and unit, and a panel unit whose managed unit names are config. Only one session runs at a time, so the socket and ports do not collide. Cutover includes "deploy v2, select v1, confirm v1 boots".
- **Hot git deploy stays**: `/dev/deploy` → `/dev/build` → unit restart → screenshot. Screenshots move to gamescope's `gamescope_control.take_screenshot` (grim's protocol is not served), which makes the core a Wayland client of `GAMESCOPE_WAYLAND_DISPLAY`; the same client serves `gamescope-action-binding` (§7).
- **Fix the Ansible pin first** (jedwards1230/homelab-ansible#320): the role pins a pre-workspace-model daemon and downgrades the running one on any run. The v2 core gets its own release stream and pin. Packaging (#144, #147) remains the end state for install, upgrade and rollback.
- **gamescope pinned ≥ 3.16.28** by the deploy role; 3.16.23 is not representative (§13 Q5). The headless CI compositor pins the same version.
- **`--expose-wayland` breaks the WSI layer for every Steam-runtime app, and the fix is per-app.** gamescope hands children `WAYLAND_DISPLAY=gamescope-0`; pressure-vessel (the Steam runtime container) preserves only `wayland-*` socket names, so it rewrites that to `wayland-0` and rewrites `GAMESCOPE_WAYLAND_DISPLAY` to an absolute socket path. The layer's `isRunningUnderGamescope()` gate can then never match, so it does not load: no HDR for any Steam-runtime app, and with dialogs enabled a black screen. Measured 2026-09-05. **The per-app fix is sufficient and was proven** — launching Steam with `WAYLAND_DISPLAY` unset worked with `--expose-wayland` still on and no session restart — so v2 keeps the flag and unsets the variable in the launch environment for that app class. Do **not** chase this with gamescope master: master's bd52d6e sets `WAYLAND_DISPLAY=""`, which pressure-vessel turns back into `wayland-0`, recreating the breakage.
- **New config** (`config.toml.example` rows and `daemon_config.rs` structs land with the core): `[display]` width/height/refresh, HDR default, SDR nits, hotplug settle; `[session]` shell app id, Xwayland count, the switch/map/launch-confirm bounds; `[supervisor]` stall seconds, restart thresholds, short-session window/count; `[av]` AVR host/port, input code, zone-2 policy, TV MAC/broadcast, webOS host and key file. Landed as `config/core.toml.example` + `core/src/config.rs` (its own file, not `config.toml` — v1's root is `deny_unknown_fields`). The `[display]` mode and `session.xwayland_count` reach gamescope through an env file the session script renders with `tv-shell-core write-session-env` and the unit reads with a required `EnvironmentFile=`; a config key whose stated consumer does not exist is the repo's #416 class and a test asserts the link. **`[[app]]` and `[session].boot_app` landed 2026-09-06** — see below.
- **App classes are configuration, and the launch ENVIRONMENT is part of the class.** Measured on hardware 2026-09-06: a bare `/usr/bin/moonlight` inside the v2 session inherits `WAYLAND_DISPLAY=gamescope-0` and `XDG_SESSION_TYPE=wayland`, so Moonlight selects native Wayland — which §6 records it segfaulting on under gamescope — and never maps a window. The base layer is set correctly and the television stays black. `[[app]]` in `core.toml` carries each class's id, argv, `env` and **`env_unset`**; the removal half is not optional, because no value substitutes for absence (`WAYLAND_DISPLAY=""` is not unset, and pressure-vessel rewrites an empty one back to `wayland-0`, above). `launch <appid>` with no command is the class form and is the default path; an explicit command for a known id still takes the class environment, since the environment belongs to the class rather than to the argv. §12's other class fields (id strategy, input contract, HDR expectation) are deliberately not modelled yet — nothing reads them.
- **The boot client fires on an observation, not on startup.** `[session].boot_app` names the class the core launches and shows once, and `core/src/boot.rs` decides from the startup reconcile alone: an EMPTY base layer **and** nothing on screen. A populated list, an app on screen, or a read that failed are all "session in use", so a core restart under a live game — the designed `Restart=always` recovery path — never relaunches or takes the screen, and never resurrects an app the user quit. It runs after the IPC socket is listening, so a cold app start cannot delay the §9 control surface.
- **The boot app is SUPERVISED, and durability is why.** A launch-once boot client is durable-once: a streaming client that wedges and dies leaves the television black until somebody reboots it, and this hardware has done that repeatedly. `core/src/boot.rs` relaunches it with the prototype's measured fast-exit backoff (`dev/gamescope/client.sh`: 3 exits inside 10 s stretches the retry from 2 s to 60 s), logs every backoff at WARN with the running count, and never gives up by default — a permanent give-up guarantees a black screen, while a 60 s retry recovers by itself once the runtime is fixed. **"The app exited" and "the core restarted" are kept apart structurally**: `supervise` requires a token constructed only by a confirmed launch from this core, so an exit is a message on a channel obtained by starting the process, never an inference from the world. A relaunch is additionally refused if anything else is on screen, and a CLEAN exit ends supervision under the default `on-failure` policy, which assumes a shell to land on. **That assumption is ahead of the code**: §13 Q1 is open and `tv-shell-gamescope-child.sh` is still `exec sleep infinity`, so on a deployment with no shell a clean quit lands on an empty compositor — a black television recoverable only from another machine. Such a deployment sets `boot_relaunch = "always"`; the default stays `on-failure` because it is correct for the design's end state and because a default that changed meaning between releases would silently change behaviour for anyone upgrading across it.
- **A restarted core adopts the running app; adoption is not a launch.** Measured on hardware 2026-09-06: after a core restart the running boot app was unsupervised permanently, so crash durability lapsed the first time the core restarted — and with `Restart=always` on the unit that is routine, not rare. It is a direct consequence of the property that makes a restart safe: `Supervised` is only constructible by a launch from this core, so a core that correctly declined to launch also adopted nothing. `boot::adopt` attaches a watcher to the app already on screen and does nothing else — no launch, no show, no base-layer write — so the "a restart never steals the screen" property is untouched by construction. The pid comes from `GAMESCOPE_FOCUSABLE_WINDOWS` (the compositor's own `XResQueryClientIds` answer) and the cgroup SCOPE is what is watched, so pid reuse cannot fool it. **An adopted app's exit status is unknowable** — `wait()` is only for a process you forked — so `ExitKind::Unknown` exists as its own variant: `on-failure` refuses to guess (guessing "crash" would relaunch over a quit, which the user cannot escape) and `always` keeps the app alive without needing to know.
- **An existing `core.toml` keeps working when keys are added.** Every section is `#[serde(default)]`, so a file missing new keys loads and takes defaults — there is no migration step. `deny_unknown_fields` is the other half: an unknown key is refused by name, so a typo can never silently run a default. A test pins both directions, because "will the file on the box still boot?" is not a question to answer from memory before a reboot.
- **Cutover criteria**: §6 table green on the pinned build, driven through the runtime atoms, including the eyes-only rows and the new bench rows; field assertions at zero and heartbeat false positives at zero for seven consecutive days of normal use; every PRD §3 journey reproduced; a Moonlight session, a Plex session and a web app each survive a shell restart underneath; the §8 site preconditions verified; v1 still boots after a v2 deploy.
- **Rollback** is selecting the v1 session at the display manager, by hand or by the short-session hook.

## 12. Streaming clients and app classes

**Two streaming clients are first-class, permanently supported, and independently testable.** The user streams over Moonlight today and expects to move to Steam Remote Play as its features catch up; Moonlight stays in the repo regardless. Both sit under one contract: a window resolved by scope or tagged by pid, one base-layer switch, HDR through the X11 WSI path, audio ownership by the base window, the `gamepad` input contract.

| | Moonlight | Steam Remote Play |
|---|---|---|
| Status under gamescope | **Proven 2026-09-05**: xcb, tag-by-pid on the stream window (its second X window, K6), HDR10 swapchain exposed to the client, 120 fps alone and with the shell over it | **Measured 2026-09-05**, Big Picture flavour: it runs and renders. Two real streams under the kit; the stream window is tagged (Steam tags it with the *game's* app id, and the kit defers via `--keep-existing`), 120 fps held, the pad reached the game. **SDR, not HDR** — Valve's client declines the HDR the compositor offers (below). Steam owns `GAMESCOPECTRL_BASELAYER_APPID` outright. Steam Link remains unmeasured (not installed on the box; only the kit's "not installed → exit 2" path is exercised) |
| Flavours | one | (a) the full Steam client in Big Picture as a tagged app, carrying the `streaming_client` window (`STEAM_STREAMING_CLIENT=1`, always a focus candidate under SteamControlled); (b) the standalone Steam Link client, a plain SDL app with no shell-role contention, HDR on Linux unverified |
| Known risks | check the host's `<currentgame>` before launching (a busy host shows an invisible dialog) | (1) SteamOS 3.9's session exports `GAMESCOPE_DISPLAY_DISABLED=1` for `streaming_client` "until buffer frozen issues are understood better", and gamescope#2196 (flicker, unpredictable input under `--steam`) is open. (2) The full Steam client under `-e` expects to be app 769 and writes `GAMESCOPECTRL_BASELAYER_APPID` itself when a stream starts, so it may fight our shell for the base layer (`steamos-39-gamescope-shell.md` §2, §6) — **measured 2026-09-05: it does, and it wins.** The kit wrote `9004,769,9001`; Steam replaced it within seconds and rewrote it on every stream start and stop (`[413091, 769]` → `[413091, 252950, 769]` → `[413091, 769]` → `[413091, 3321460, 769]`), dropping the kit's own id from `GAMESCOPE_FOCUSABLE_APPS` entirely. (3) **Remote Play is SDR by Valve's gate.** The WSI layer hooked `streaming_client` and logged `server hdr output enabled: true` / `hdr formats exposed to client: true`; the client created a `VK_FORMAT_B8G8R8A8_UNORM` / `VK_COLOR_SPACE_SRGB_NONLINEAR_KHR` swapchain on the next line. HDR was offered and declined — no compositor, flag, pin or kernel changed it on that build. Moonlight remains the HDR path. **Scope corrected 2026-09-14** (§13 Q3): this is one *receiver* declining one offer, not a general Steam/gamescope-on-Linux gate — local games under gamescope get HDR, and this box runs Moonlight at 4K120 HDR10 — and the result predates nine days of client shipping across a `steamdeck_stable`/`publicbeta` toggle, so it is pending re-measurement. (4) `--expose-wayland` silently disables the WSI layer for every Steam-runtime app (§11); run Steam with `WAYLAND_DISPLAY` unset. (5) Sunshine and Remote Play must not target the same streaming host at once (§9 interlock) |
| Gate | kit criteria 3 and 8 (pass) | **kit criterion 10**: stream shown, **HDR on the TV, or SDR recorded as such**, 120 fps, pad reaches the game. Two of the five conditions as first written are now disproven rather than merely unmet: HDR is unreachable on this client (a Valve-side gate, below), and "no base-layer fight" is dead — Steam wins that atom, so it is a design requirement on the core (§9), not a pass condition. It still decides which flavour is supported and is §13 Q3 |

Other app classes:

| Class | Launch | Id | Notes |
|---|---|---|---|
| Plex HTPC (native) | own server; `keyboard` contract | scope | Keep its CEC disabled; it runs under `bwrap`, which must keep the host pid namespace or scope and pid both fail |
| Chromium `--app` web apps | own server, own `--user-data-dir`/`--class`; `keyboard` contract | scope, class fallback | §13 Q4 |
| Home Assistant, music streaming | later, as plugins | | §13 Q2: requirements only |

Plugin requirements (mechanism undecided): a plugin declares an app class, a launch command, an id strategy (scope, pid, or class), an input contract (`gamepad` / `keyboard`), an HDR expectation, and optional home-widget manifests; it never writes compositor atoms itself.

### Fallback considered

If criterion 10 fails, or if a Steam-first future makes a custom shell not worth its cost, the fallback is a **Bazzite/ChimeraOS-style `gamescope-session` with Steam Big Picture as the shell**. It buys Remote Play, Steam Input, the overlay and HDR for free, with Moonlight and Plex as non-Steam shortcuts. It costs what this design exists to keep: our shell as a peer of apps, native Plex and web apps, and daemon-owned AV control, which would move into a sidecar or a Decky-style plugin. It is a real path, not a strawman, and criterion 10 is where it is chosen. **Criterion 10's Big Picture half passed on 2026-09-05**, so this fallback is not selected; it stays on the page as the named alternative should a Steam-first future make a custom shell not worth its cost. Either way the ChimeraOS session files are reused (§4).

### What §9 describes and what `core/` implements

§9 is the design. As of 2026-09-06 the shipped `core/` crate and its units cover
part of it, and the gap is recorded here rather than left to be discovered on the
couch:

| §9 row | State in `core/` |
|---|---|
| gamescope dies → session exits | `BindsTo=` on the target, plus the session script's `--wait`. Untested on hardware |
| Core dies → restart, stateless, never writes "home" on boot | Implemented (`reconcile_on_start`) |
| Stuck in an app → return to the shell | `home` and its bounded verify are implemented. The **escape that triggers it** is not: that needs the passive pad reader of §7, which is not in the crate |
| Frames stop → forced-paint heartbeat | **Not implemented.** `[supervisor].stall_secs` has no reader |
| Shell crash-loops → short-session tracker, then select v1 | **Not implemented.** No `ExecStartPre`/`ExecStopPost`, no counter, no deployment hook; `[supervisor].restart_threshold` / `restart_window_secs` have no reader. The gamescope unit briefly carried `StartLimitIntervalSec=60`/`StartLimitBurst=3` with a comment claiming to deliver this — the limiter was inert (`Restart=no`, so there is never a second attempt to count, and the session script's `reset-failed` clears the counter on every relogin) and has been removed rather than left standing as a protection that is not there. **Rollback is manual: select the v1 session at the display manager.** |
| Shell dies → restart under the live compositor | The `Upholds=` is in the target; the shell unit does not exist yet |
| Compositor wedged but alive → panel restarts units | **Not implemented, and the target no longer pretends otherwise.** The panel was `Wants=` on the session target, so the recovery surface would have died with the thing it recovers; it now belongs to `default.target` when it lands. Until then the v2 session has no recovery surface |
| Only one supervisor holds restart authority | **Not resolved.** The units carried `Conflicts=tv-shell-input.service`, which is bidirectional — so the Ansible CEC watchdog restarting `tv-shell-input` on a bad `cec-health` reading would have stopped the v2 session target and black-screened a live game. Exclusion is now one-directional (the session script stops and `mask --runtime`s the v1 units). That is a mitigation; §9's actual requirement — the watchdog stands down at cutover and its probe distinguishes a stopped unit from a wedged adapter — is still unfiled on the Ansible side |

§5's **transient-unmap hysteresis** is likewise unimplemented:
`GAMESCOPECTRL_BASELAYER_WINDOW` is read into `ScreenState` and never written,
and nothing holds the base layer across a known transition or applies hysteresis
before treating a fallback to the shell as an app exit.

Two §5 rules the crate does implement in a shape §5 does not spell out, both
because a single bound conflated two different waits:

- A `show` verifies against **two** bounds — `switch_timeout_ms` once the target
  has a mapped window, `map_timeout_ms` while it has none — and reports two
  distinct errors. Under one bound a `show` issued right after a `launch` failed
  on every working launch, which trains a caller to ignore the one error §5 says
  must never be ignored.
- A `launch` **confirms itself** (launcher alive, `/proc/<pid>/cgroup` naming the
  scope) before reporting success. `Command::spawn` returning `Ok` says nothing
  about whether the app started, so an unconfirmed launch is an error rather than
  a success payload naming a dead pid.

## 13. Open questions

Most of these now carry answers, and the numbering is kept anyway — a question's number is
referenced from §12, §14 and from issues, so renumbering on resolution would break the record
this section exists to keep. Each entry preserves the reasoning and the measurements that led
to its answer; resolutions are appended and marked inline, never substituted for the argument.
As of **2026-09-14**, Q2 and Q3 are the two that are still not settled — Q2 deliberately
deferred, Q3 reopened — and Q9 is the one blocked on evidence rather than on judgement. The
rest are answered.

1. **Shell runtime: Quickshell on xcb, or a plain Qt Quick application?** The two are not of equal standing and the asymmetry belongs on the record. §6 measured the plain `qml` runtime (Qt 6) working as the prototype shell under gamescope at 4K120; Quickshell's xcb operation is unverified, and its headline feature — layer-shell — does not exist under gamescope at all, so choosing it buys nothing the plain runtime lacks and carries an unmeasured risk. Either way the shell must set X11 properties on its own windows before map (a C++ helper or plugin) and split drawer, QAM and toasts into separate toplevels.

   The trade was assumed to be a port versus a rewrite; measured, it is neither. Of the 124 QML files in `shell/`, 34 import Quickshell at all, only 4 touch `PanelWindow`/`WlrLayershell`, and `Socket` use is confined to the single `SocketClient.qml`. The bulk is `Process` — 50 sites across 16 files — much of which §4 already moves into the core regardless of this answer. So the cost is a shim exercise: roughly three helper types plus the four window files, not a rewrite of the UI.

   **Still open**, because the evidence above is static analysis and a prototype, not the real shell on the real box. What closes it is a spike on hardware: build the shim, run today's `shell/` against it under gamescope on the pinned build, and confirm the self-tagging, the separate overlay toplevels and the pad path. §13 Q12's v2-`shell/` half waits on the same spike.

   **The shim exists as of 2026-09-07** (`shell-v2/`, [`V2_SHELL.md`](V2_SHELL.md)), and building it moved the question rather than closing it. Two findings:

   - **Neither candidate runtime can be driven as-is.** `QWindow::setVisible` is **not virtual** in Qt 6, nor is `create()`, and there is no other virtual called between window creation and map — so there is no hook to override in *any* Qt-based shell, Quickshell included. The shim enforces the ordering by redeclaring the `visible` Q_PROPERTY on a `QQuickWindow` subclass, hiding the base `setVisible`, deleting the `show()` family, and deferring visibility to `componentComplete()`. That is a C++ type either way, which is why the answer is a plain Qt Quick **application** rather than the plain `qml` runtime: once a C++ type is required, a build step is required, and owning `main()` is then free.
   - **The tagging contract is now testable without gamescope.** The role→atom mapping is a pure function with its own no-display test, and the ordering is asserted from a second X client against a real X server in CI — `PropertyNotify` before `MapNotify`, per role.

   **Answered on hardware, 2026-09-09.** `shell-v2` was built from main and run on the box as an ordinary client under the live gamescope session beside a running Moonlight — no unit change, no boot-path risk. It maps; gamescope resolves it to the shell app id (9001), which is only possible if the property landed **before** the map, since post-map tagging is measured non-functional on this build (#472); the core lists it in `focusable_apps`; `show 9001` puts it on screen; and it renders correctly at 3840x2160 with live core state. The drawer opens as a **separate toplevel** carrying `STEAM_OVERLAY=1` + `STEAM_INPUT_FOCUS=1` and no `STEAM_GAME`, and **gamescope grants it the keyboard** while `GAMESCOPE_FOCUSED_WINDOW` correctly stays on the base. So the plain Qt Quick application is the answer, and §7's premise that the compositor decides focus holds.

   Two things the same run found, both narrower than the runtime question:

   - **Focus is not returned when the overlay closes**, and the display then freezes on its last frame — #481. The grant works; the handback does not.
   - **The pad path is still unexercised.** Keys were synthesised with `xdotool` (XTEST), which may take a different route through gamescope than a real evdev event; and the v2 input core is default-off and undeployed.

   §13 Q12's v2-`shell/` half is unblocked by this. `V2_SHELL.md` §8 lists what remains unproven.
2. **Plugin mechanism** for Home Assistant and music streaming: manifest-driven `.desktop` extensions, a core-side registry, or out-of-process plugins over the IPC.

   **Deferred 2026-09-14**, deliberately and without picking a mechanism. A plugin needs a shell to plug into, and the shell currently has no install path at all: `/opt/tv-shell-v2/bin/` holds `tv-shell-core` and two gamescope scripts, `scripts/install-v2.sh` never touches `shell-v2/`, and the session target references a `tv-shell-v2-shell.service` that does not exist. Choosing between a `.desktop` extension, a core-side registry and out-of-process IPC plugins on that basis would be choosing an integration surface for a host that cannot be run, and the requirements paragraph below is deliberately all this question has. Revisit once there is a UI on the couch.
3. **Steam Remote Play under gamescope — the Steam Link half (kit criterion 10).** Partly answered 2026-09-05: **Big Picture as a tagged app works**, in its SDR form. The stream window is tagged (with the game's app id, by Steam itself), 120 fps held, input reached the game, and the base-layer question is settled against us (§9). What is still open is the **standalone Steam Link client**, which is not installed on the box, so only the kit's "not installed → exit 2" path is exercised: whether it is worth supporting beside Big Picture, and whether its HDR on Linux is any better than Big Picture's — the latter is capped by a Valve-side gate on `streaming_client`, so the standalone client is the only remaining place an HDR Remote Play could come from. The consequence of the SDR result is the live question: Remote Play is an SDR path for as long as Valve's client declines the HDR swapchain, which is a product decision (Moonlight stays the HDR path) rather than a compositor one. gamescope#2196 and the v1 finding that Steam's capture-window selection is a Valve-side bug stay open. A failure of the remaining half narrows the supported flavour; it does not select the §12 fallback, since Big Picture passes.

   **Reopened 2026-09-14, on both halves, with a correction to what 09-05 actually measured.** The paragraph above — and the §12 table and decision log with it — reads as "Remote Play is SDR on Linux by a Valve-side gate". That is broader than the evidence. What was measured is narrower: the Remote Play **receiver** (`streaming_client`) declined an HDR swapchain that gamescope was offering it. The WSI layer logged `server hdr output enabled: true` / `hdr formats exposed to client: true`, every swapchain the client then created came back `VK_FORMAT_B8G8R8A8_UNORM` / `VK_COLOR_SPACE_SRGB_NONLINEAR_KHR`, and `streaming_client.log` said in as many words that it was "using the sRGB colorspace" — including under the Deck flags `-steamos3 -steampal -steamdeck -gamepadui`. That is one client, on one build, declining one offer. It is **not** evidence that Steam or gamescope HDR is broken on Linux generally: local games under gamescope get HDR fine (it is what SteamOS and the Steam Deck do), and htpc-1 itself runs Moonlight at 4K120 HDR10 through the same compositor and the same WSI layer.

   Two further corrections to the scope of that run. First, 09-05 **was already the full Steam client path** — Big Picture's Remote Play is `streaming_client` — so "test the full client next" is not the open work; the genuinely untested flavour is the **standalone Steam Link client**, still not installed on the box. Second, the result is now nine days old on a client that ships continuously, and the session had toggled between the `steamdeck_stable` and `publicbeta` betas during the test, so it is not safe to assume it describes the build a user would get today.

   What closes it is a re-measurement of **both** receivers — the full Steam client in Big Picture and standalone Steam Link — on current builds, with the beta channel recorded rather than drifting. Until then the product decision stands unchanged for planning purposes (Moonlight is the HDR path), because it is the measured state; it is the *generality* of the claim that is withdrawn, not the observation.
4. **Chromium: Xwayland or native Wayland, and per-app profiles.** Native has no focus selector under SteamControlled; Xwayland is the safe path but hardware decode and Widevine under Xwayland on this GPU are unmeasured, and the per-app `--user-data-dir` split costs shared logins.

   **Answered 2026-09-14: Xwayland, with a SHARED Chromium profile** — one `--user-data-dir` across all web apps, not one per app. Neither half was close. Xwayland is barely a choice under gamescope (see the v3 note below), and the profile split was costing the one thing the couch cannot pay for: a keyboard. A shared profile means sign in once, on whatever input device is to hand, and every web app is signed in; per-app profiles mean signing in again per app, on a television, with a gamepad. `--class` remains the per-app knob for window identity, which is what §12's id strategy actually needs from the split.

   **A goal this answer makes explicit: Wayland-native clients are a v3 goal, not a v2 one.** They are deferred under gamescope for three reasons, each measured rather than assumed:

   - gamescope's WSI layer **hardcodes HDR off for native-Wayland clients**. This is why Moonlight runs on xcb (§12), and it applies to any client we might otherwise have run native.
   - The entire v2 window and focus model is **X11 atoms**: `GAMESCOPE_FOCUSABLE_WINDOWS`, `STEAM_GAME` tagging, and `shell-v2`'s pre-map shim, which reaches the display through `QNativeInterface::QX11Application`. A native-Wayland client is outside every one of those mechanisms.
   - gamescope implements **no Wayland capture protocol**, which is why screenshots go through an X root property (Q13).

   For accuracy, since "Xwayland not Wayland" invites the wrong mental model: **gamescope IS itself a Wayland compositor**, and clients run under the Xwayland server it starts. Wayland is the substrate throughout; it is the *client protocol* that is X11, and it is X11 because the compositor's own contracts — HDR, focus, capture — are expressed there.
5. **gamescope pin and packaging.** Arch's package at the 3.16.28 tag, a built tag, or tracking master for content-driven HDR (commit 6513879, not in any tag).

   **Answered 2026-09-14: hold the Arch `3.16.28-1` pin.** The explicit trigger to reopen is narrow and checkable: **an upstream *tag* that contains commit `6513879`** (content-driven HDR). Not master containing it — it already does — a tag.

   The rationale is cost against gain, and the gain is the smaller side. Content-driven HDR is a refinement of something already measured working: HDR10, BT2020, PQ at 4K120 is on the record from the 09-05 bench. The cost is a fork we would own indefinitely: building our own tag plus cherry-picking `6513879` means rebasing on every upstream tag, and carrying a patch to dodge `bd52d6e` — which reintroduces the Steam WSI breakage — that upstream may well fix themselves, at which point the patch is pure debt. On top of that every pin move costs a full live bench run on the box, because that is the only thing that has ever caught a regression here. Waiting for a tag converts all of that into a package bump.
6. **Kernel colour pipeline.** Stay on the stock kernel (composite path, measured fine for a switch) or ship one with `CONFIG_AMD_PRIVATE_COLOR` for direct scanout; decide after the overlay-over-HDR bench row and the TV panel's bit-depth reading.

   **Answered 2026-09-14: stay on the stock kernel.** Revisit only on a real observed symptom — a visible artefact on the couch, not a number in a report.

   This answer knowingly accepts an unresolved fact: **whether the output is 8-bit or 10-bit remains unconfirmed**, because bit depth is unmeasurable on this kernel (09-05; `debugfs` `output_bpc` carries no `Current:` line here even as root). The decision does not wait on it, because the thing that would resolve it — a custom kernel — is the thing being declined. A custom kernel on an appliance is a large, permanent maintenance surface: a rebuild per kernel update, on a box whose whole point is that it turns on and works. Against that, the composite path measured fine for a switch, and no symptom has been observed that direct scanout would fix.
7. **CEC driver for the sidecar.** Kernel `pulse8-cec` via a `cec-rs` replacement or a small `cecd`-style daemon; whether the adapter stays in the chain at all if the AVR's push events prove sufficient.

   **Answered 2026-09-14 — and this AMENDS the 2026-09-04 decision**, which made IP the AV authority and CEC a passive observer. That ordering is reversed. **CEC is the PRIMARY AV control backend**: kernel `pulse8-cec`, in its own dedicated daemon — the `tv-shell-v2-cec.service` that the session target already references and that does not yet exist — owning power, input-switching and volume. `libcec`/`cec-rs` stay dropped, which is the one part of the 09-04 decision that survives intact. Denon telnet and LG webOS/WoL **remain implemented but are DEMOTED to a recovery path**, used when the CEC bus is unavailable or the adapter has wedged.

   Three reasons, in the order they carried weight:

   - **CEC is vendor-neutral; IP control is not.** The IP path is Denon-specific telnet plus LG-specific webOS auth that has already broken across firmware revisions. Swap either box — and this is an AV rack, boxes get swapped — and the IP path is bespoke work again, whereas CEC is a bus contract the next receiver also speaks.
   - **The reliability record blames libcec, not CEC.** This is the crux, and it is why the 09-04 decision looks wrong in hindsight rather than merely superseded. Every failure in the ops record — the adapter wedging, `cec-health` reporting health it could not know, Plex grabbing `/dev/ttyACM0`, a watchdog that "recovered" a daemon three times that was never broken — is a **libcec/watchdog** problem. None of them is a CEC *protocol* problem. The kernel `pulse8-cec` driver plus the kernel CEC API is a far thinner surface than libcec, and the watchdog that caused most of the false recoveries is not part of this design at all.
   - **The IP fallback is worth building precisely because it retires `jedwards1230/tv-shell#251`.** Today a true adapter wedge costs a reboot. With an IP recovery path it costs a degraded mode that is recoverable in place — the television still responds, through a worse door. That is the whole value of keeping the Denon and LG code: not as the authority, as the thing that makes a wedge survivable.

   **Its own daemon rather than in-core**, for the same reason: the wedge history is real, and the core is what keeps Moonlight alive. A CEC backend that hangs must not be able to take the compositor's supervisor down with it.

   **Amended 2026-09-14, later the same day, by building it.** The clause above — "used when the CEC bus is unavailable or the adapter has wedged" — is **too narrow**, and appending the correction rather than editing it in place is the point: the original wording is what the implementation was reviewed against.

   The IP leg is a **capability complement on the cold path** and a **failover on the warm path**. Two operations have no CEC expression at all, so they run over IP with a perfectly healthy bus: the receiver's **Zone 2** (`Z2OFF` has no CEC equivalent, so it goes out on every standby where Zone 2 matters) and a **cold wake of a television at mains standby** (`<Image View On>` reaches nothing; that needs Wake-on-LAN, as does the AVR when its network-control-in-standby setting is off). `cec/src/failover.rs` therefore decides only the *warm* authority, while the wake and standby sequences fire their IP steps *before* the CEC steps unconditionally. §8's table carries the same correction.

   One thing in this entry's own framing did not survive contact either: "Denon telnet and **LG webOS**/WoL remain implemented". Neither was implemented — the Denon client existed only in a closed, never-merged PR and has now been ported to `cec/src/ip/avr.rs`, and **no LG webOS/SSAP client has ever existed in this tree**. The television's IP leg is **Wake-on-LAN only, write-only, with no state read**, and it should stay that way: a webOS client is a second auth surface with a history of breaking across firmware, which is one of the three reasons this entry demoted IP.

   **Measurement taken on htpc-1 today, 2026-09-14, which sets the starting line**: `pulse8-cec` is **not loaded**, and there is **no `/dev/cec*` node at all**. The `cec` core module is loaded, but only because amdgpu pulls it in — not because anything is using the adapter. The Pulse-Eight adapter is sitting as a raw serial device at `/dev/ttyACM0`, held by nobody (`fuser` returns no holder). So **loading `pulse8-cec` to obtain a `/dev/cecN` is step one of this work**, before any daemon code is written; there is currently no kernel CEC device for a daemon to open.
8. **Privilege model.** The panel's exec tier, the short-session fallback (a root-owned display-manager file), and the pacman path: sudoers allowlist (v1) or polkit + a system D-Bus helper (SteamOS shape). PRD §4 lists display-manager setup as a non-goal; the fallback hook either lives in deployment or amends that.

   **Answered 2026-09-14, as a sequence rather than a choice: sudoers allowlist NOW, polkit + a system D-Bus helper LATER.** The allowlist is v1's shape, it already works on this box, and it is trivially auditable — the set of permitted commands is a file you can read in one sitting. Polkit plus a system D-Bus helper is the better end state and the SteamOS shape, and it is still the intended destination; it is simply not worth spending the weeks now.

   The sequencing is the decision, not a hedge. The critical path is the shell reaching the couch (Q2's blocker), and a privilege-model rewrite is a self-contained project that would sit squarely across it while changing nothing a user sees.
9. **Cause of the launch-coincident hotplug**: the AVR, an audio infoframe renegotiation at stream start, or the v1 CEC lifecycle. Discriminating runs: CEC off; Moonlight audio disabled. The relaunch-once policy waits on the answer.

   **Still open, with a partial result from 2026-09-14 and a reason it could not be completed.** The approach is to check the **free** discriminator first — the one that costs no test run, because the box has been sitting in that configuration anyway. v2 has been running with **zero CEC involvement**: no `/dev/cec*`, `pulse8-cec` unloaded, and nothing holding `/dev/ttyACM0` (Q7's measurement). If a launch-coincident hotplug were observed in that state, the v1 CEC lifecycle would be exonerated outright and the field would narrow to the AVR or the audio infoframe.

   **The check is inconclusive**, and the reason is retention, not the hypothesis. journald on htpc-1 only reaches back to 2026-09-13 16:54 (43.8 MB on disk), so roughly **21 hours** could be surveyed rather than the eight days v2 has been running — and a survey of 21 hours that happened to contain no stream start proves nothing about eight days that did. To conclude, a stream start has to be **observed inside the current retention window**, or retention has to be increased so the window covers ordinary use.
10. **Headless CI feasibility** on a hosted runner: lavapipe acceptance, stats emission, `GAMESCOPE_CREATE_XWAYLAND_SERVER` under headless.

   **Answered 2026-09-14: spike it.** Worth the attempt, on a specific and unflattering piece of evidence.

   The offline fixtures use a **fake `xprop`**. They therefore test our logic and never once test the compositor's behaviour — which is exactly how **126 assertions passed green while the box was entirely broken on the 3.16.28 pin**. That is not a gap in coverage, it is a category of bug the current suite structurally cannot see, and a real gamescope in CI is the only thing that catches it.

   The open risk is recorded rather than argued away: **lavapipe may simply not cooperate on a hosted runner** — that is the whole reason this is a spike and not a task. If it does not, the result is a documented negative and the fixtures stay what they are, honestly labelled.
11. **VRR default**: on, off, or per-app, given the OLED near-black flicker and AVR OSD notes in the ops record. **Still open**, and now a config key (`[display].vrr`) rather than a hardcoded `--adaptive-sync` in the unit — the unit was answering this question in the direction the ops record warns against, without saying it was answering anything. The default matches the measured session (on); the ops record's concern is untested against it.

    **Answered 2026-09-14: PER-APP, defaulting to on.** Neither global answer fits, because the risk and the benefit do not live in the same place. The OLED near-black flicker the ops record warns about is worst on **static near-black UI** — which is the shell's own drawer and QAM, and video letterboxing — while VRR earns its keep on exactly the content that is neither static nor near-black: Moonlight and games. So VRR on for streaming clients and games, off for the shell and for video, and the global `[display].vrr` key becomes the default rather than the policy.

    This needs the **`[[app]]` config plumbing** — a per-app table with a VRR field, which does not exist yet. Until it lands the effective behaviour is the current global default (on), which is the measured session.
12. **Core name and repo layout**: whether `daemon/` evolves in place or a new crate is created beside it while v1 is kept buildable; the same for a v2 `shell/`. **Answered 2026-09-06 for the core half**: a NEW crate, `core/` (`tv-shell-core`), beside `daemon/`, with v1 (`daemon/`, `host/`, `protocol/`, `panel/`) untouched and still building. Evolving `daemon/` in place was never viable at the config layer — its root is `deny_unknown_fields`, so a v2 table in `config.toml` aborts v1 at startup — so the core takes its own file, socket and units (§11). The **v2 shell half is answered 2026-09-07** in the same shape: a new tree, **`shell-v2/`**, beside `shell/`, with its own CMake build, its own single QML module and its own CI area, and v1's `shell/` untouched and still booting the couch. `shell/` is not ported and is not deleted.
13. **Screenshot fidelity under HDR** through `gamescope_control` (`screen_buffer` type) versus a WSI-side capture.

    **Answered 2026-09-14, by reordering it: fix REACHABILITY first, fidelity later.** The question as written asks which capture path is more faithful under HDR. The problem actually being felt is cruder than that — v1's capture was reachable over HTTP and MCP, and **v2's capture verb is on-box only**, so every piece of v2 UI work is being done blind by anyone not sitting at the television.

    So: keep the X root-property capture exactly as it is (it is what gamescope offers — see Q4 on the absent Wayland capture protocol), and **expose it off-box through the panel**. That restores the v1 property that made UI iteration possible at all. The fidelity question — `gamescope_control`'s `screen_buffer` versus a WSI-side capture — is explicitly **deferred behind it**, because a more faithful image nobody can retrieve is worth less than an imperfect one that reaches the person doing the work.

## 14. Decision log

| Date | Decision | Where |
|---|---|---|
| 2026-05-24 | Hyprland + Quickshell layer-shell; gamescope rejected for "no layer-shell" | PRD §5 |
| 2026-07-04 | Kiosk model on Hyprland: declarative isolation + daemon hardening; gamescope re-rejected | #307, #308 |
| 2026-08-29 | Nested per-app gamescope rejected; SteamOS itself rejected | "TV Shell vs SteamOS" artifact |
| 2026-09-04 | v2 beside v1; one Rust core, shell as ordinary client; in-repo supervisor with frame heartbeat (reverses PRD §12.5); grab only while shell visible; headless CI + field assertions; keep hot deploy, fix the Ansible pin; panel in scope; plugins later; IP is the AV authority with a kernel-CEC observer, libcec goes; theater sleep only when the HTPC owns the display; TV remote is a goal | memory note; #453; homelab-ansible#318 |
| 2026-09-04 | Compositor gated on a one-week gamescope measurement; fail rule: 10-bit HDR at 4K120 or composite cost | `dev/gamescope/README.md` |
| 2026-09-05 | Phases 1–3 pass; bit depth unmeasurable on this kernel; bare `gamescopectl <convar>` resets the convar; Moonlight must be xcb; tag by pid | #454, `gamescope-hdr-feedback.md` |
| 2026-09-05 | **gamescope is the v2 compositor**, pin ≥ 3.16.28; Hyprland stays v1-only; Smithay dropped; SteamOS unit shape and base-layer contract adopted; wiki writes held until this document exists | this document |
| 2026-09-05 | Moonlight and Steam Remote Play are both first-class and permanently supported; Remote Play under gamescope is kit criterion 10 and gates the flavour (Big Picture vs Steam Link); a Steam-as-shell `gamescope-session` is the named fallback; the shell's app id is private, 769 stays Steam's | this document, `dev/gamescope/README.md` |
| 2026-09-05 | **Kit criterion 10 measured.** Steam Remote Play runs under the prototype: Big Picture as a tagged app passes in its **SDR** form (120 fps, pad reaches the game, stream window tagged with the game's app id and deferred to). Remote Play is **SDR by a Valve-side gate** — the client declines the HDR swapchain the compositor offers — so Moonlight stays the HDR path. **Steam owns the base-layer atom** and rewrites it per stream start and stop; the core reconciles after Steam rather than contending. Steam Link is **unmeasured** (not installed). `--expose-wayland` disables the WSI layer for Steam-runtime apps; the per-app `WAYLAND_DISPLAY`-unset fix is proven and the flag stays. The decision rule (criteria 1 and 3) is untouched and still passes — gamescope remains the v2 compositor | this document, jedwards1230/tv-shell#458 |
| 2026-09-05 | Review findings folded in: `app-steam-app` scope prefix is the upstream contract and the primary id; shell self-tags; the core is stateless and reads the base-layer list back on restart; v1 and v2 share no config file, prefix or unit name; the heartbeat is a forced-paint probe with one FIFO reader; `GAMESCOPE_FOCUSED_WINDOW` not `_APP` is the truth under an overlay; presenters collapse to `gamepad`/`keyboard` with persistent uinput devices | this document |
| 2026-09-06 | **§13 Q12 answered for the core**: a new crate `tv-shell-core` in `core/`, beside `daemon/`, not an evolution of it — v1 (`daemon/`, `host/`, `protocol/`, `panel/`) is untouched and still builds, and the two share no config file (`core.toml`, since v1's root is `deny_unknown_fields`), socket (`tv-shell-core.sock`) or unit name. The crate lands with the typed atom layer, `ScreenState`, scope launching, base-layer show/home and the IPC server; input, CEC, the network surfaces, the heartbeat and the v2 `shell/` are not in it | this document, `core/README.md` |
| 2026-09-07 | **The v2 shell is a plain Qt Quick application in a new tree, `shell-v2/`, and it reverses two repo rules.** The pre-map self-tagging §5 requires cannot be expressed in QML, and Qt 6 offers no virtual hook between window creation and map, so the shell needs a C++ type — hence CMake, hence a binary. Consequences recorded rather than slipped in: `CLAUDE.md`'s "no build tooling" rule now scopes to v1's `shell/`, and v2's shell is **not rsync-deployable** (the deploy story is flagged, not solved). Established with it: ONE QML module (against v1's eleven registries and 69 relative-dir imports); a **role-typed `Surface`** so an overlay drawn inside the base window is unrepresentable; and a **declarative focus graph** replacing v1's eight-member duck-typed contract, in which disabling a widget cannot strand focus. Placeholder pixels, deliberate structure; nothing has met gamescope | [`V2_SHELL.md`](V2_SHELL.md) |
| 2026-09-07 | **The `intent` control surface is retired in v2, not ported.** It bundled three unrelated concerns — core operations, shell-local navigation, and synthetic keypresses — behind one flat string vocabulary; §7 splits them by owner, and the wedged-UI escape becomes a core-side base-layer write that works with the shell dead, which is what `intent home-hold` was reaching for and never had. The one property worth keeping — a closed, enumerable vocabulary — is now the `Command` enum in `core/src/protocol.rs`, enforced by exhaustive match rather than by naming convention. Recorded with it: illegal states unrepresentable, an event stream that publishes state rather than deltas, one wire format, one writer per piece of state. §13 Q1 is sharpened but **not** answered — the plain `qml` runtime is the measured one and the port is a shim, but a hardware spike closes it | this document |
| 2026-09-14 | **§13 Q4, Q5, Q6, Q8, Q10, Q11, Q13 answered; Q2 deferred; Q3 reopened.** Chromium on **Xwayland with a SHARED profile** (the couch has no keyboard), and **Wayland-native clients become a v3 goal** — gamescope's WSI hardcodes HDR off for native clients, the whole window/focus model is X11 atoms, and there is no Wayland capture protocol. **Hold the `3.16.28-1` Arch pin**; reopen only when an upstream *tag* contains `6513879`. **Stay on the stock kernel**, accepting that bit depth stays unconfirmed. **sudoers allowlist now, polkit + system D-Bus helper later**, sequenced so the shell keeps the critical path. **Spike headless CI** — the fake-`xprop` fixtures let 126 assertions pass on a wholly broken box. **VRR per-app, defaulting on** (needs `[[app]]` plumbing). **Screenshot reachability before fidelity** — expose the core's capture through the panel. **Plugins deferred** until the shell has an install path. **Remote Play reopened**: the 09-05 SDR result was the *receiver* declining an offered HDR swapchain, not a general Linux gate, and it is 9 days old across a beta toggle | this document |
| 2026-09-14 | **§13 Q7 AMENDS the 2026-09-04 AV decision: CEC is the PRIMARY control backend, not the observer.** Kernel `pulse8-cec` in a dedicated `tv-shell-v2-cec.service` owns power, input switching and volume; `libcec`/`cec-rs` stay dropped; Denon telnet and LG webOS/WoL remain implemented but are **demoted to a recovery path**. CEC is vendor-neutral where IP control is bespoke per box; and every reliability failure on the record (adapter wedging, lying `cec-health`, Plex grabbing `/dev/ttyACM0`, phantom watchdog "recoveries") is a **libcec/watchdog** failure, not a CEC-protocol one. The IP fallback is built to retire `jedwards1230/tv-shell#251`: a wedge becomes a degraded mode instead of a reboot. Measured on htpc-1 the same day: `pulse8-cec` **not loaded**, **no `/dev/cec*` node**, `cec` present only via amdgpu, adapter idle at `/dev/ttyACM0` with no holder — loading the driver is step one | this document, jedwards1230/tv-shell#251 |

## 15. References

Repository: `docs/PRD.md`, `docs/KIOSK_WINDOW_MODEL.md` (v1 model, historical), `docs/INPUT_AND_STATE.md`, `docs/IPC_PROTOCOL.md`, `docs/CONTROL_SURFACE.md`, `docs/OBSERVABILITY.md`, `docs/PANEL.md`, `docs/SYSTEMD_SETUP.md`, [`docs/v13-issue-drafts.md`](v13-issue-drafts.md) (unfiled issue drafts for the §13 actionable work), `dev/gamescope/README.md`; PRs jedwards1230/tv-shell#191, #352, #444, #448, #453, #454; issues #383, #402, #436, #455; jedwards1230/homelab-ansible#318, #320.

Research reports (outside the repo): `arch-map.md`, `gh-patterns.md`, `git-churn.md`, `ops-surface.md`, `history-sweep.md`, `htpc-stream-postmortem.html`, `gamescope-eval.md`, `cec-history.md`, `gamescope-live-measurements-2026-09-05.md`, `gamescope-hdr-feedback.md`, `steamos-39-gamescope-shell.md`, `steamos-history.md`, `tv-shell-vs-steamos-artifact.html`, `pr453-review-fixes.md`.

Upstream:

- gamescope, tags 3.16.23–3.16.28 and master: https://github.com/ValveSoftware/gamescope — `src/steamcompmgr.cpp` (focus policy, atoms, HDR feedback, stats cadence), `src/Backends/DRMBackend.cpp`, `src/Backends/HeadlessBackend.cpp`, `layer/VkLayer_FROG_gamescope_wsi.cpp`, `src/Utils/Process.cpp` (the `app-steam-app%u-%d.scope` parser), `src/convar.h` + `src/Apps/gamescopectl.cpp` (the convar reset), `protocol/gamescope-control.xml`, `protocol/gamescope-action-binding.xml`; issues #1887, #2075, #2196, #2261, #2051
- SteamOS session package `gamescope-3.16.26-2` (units, `gamescope-session`, `steam-launcher`, short-session tracker): https://github.com/Jovian-Experiments/PKGBUILDs-mirror/tree/jupiter-main/gamescope-3.16.26-2 ; SteamOS Manager: https://github.com/evlaV/steamos-manager ; ChimeraOS: https://github.com/ChimeraOS/gamescope-session
- Steam Input udev rules: https://github.com/ValveSoftware/steam-devices/blob/master/60-steam-input.rules ; InputPlumber: https://github.com/ShadowBlip/InputPlumber ; ds-inhibit: https://gitlab.com/evlaV/ds-inhibit
- Moonlight HDR decision: https://github.com/moonlight-stream/moonlight-qt/blob/master/app/streaming/video/ffmpeg-renderers/plvk.cpp
- AMD private colour properties: https://melissawen.github.io/blog/2025/05/19/drm-info-with-kms-color-api
| 2026-09-14 | **The television's IP leg is Wake-on-LAN only — no LG webOS/SSAP client, now or later.** §8 had promised "webOS for state and standby" since 2026-09-04. No such code has ever existed in this tree: no pairing key, no SSAP client, nothing. Rather than write one, the promise is withdrawn. WoL is the only IP operation the television needs that CEC cannot do (`<Image View On>` reaches nothing at mains standby), power-off is CEC's addressed `<Standby>`, and a webOS client would be a second auth surface with a documented history of breaking across firmware — which is one of the three reasons Q7 demoted IP in the first place. So the TV's IP leg is write-only with no state read (`cec/src/ip/wol.rs`), and §8's webOS-pairing-key site precondition is deleted rather than left standing | this document §8, §13 Q7; jedwards1230/tv-shell#504 |
| 2026-09-14 | **§13 Q7's "IP only when the CEC bus is unavailable or the adapter has wedged" is corrected: the IP leg is a capability COMPLEMENT on the cold path and a failover on the warm path.** Two operations have no CEC expression at all — the receiver's **Zone 2** (`Z2OFF`), and a **cold wake of a television at mains standby** — so those run over IP with a perfectly healthy bus, and the wake/standby sequences fire their IP steps *before* the CEC steps unconditionally. `failover.rs` decides only the warm-path authority. Appended to Q7 rather than substituted into it, per §13's own rule. Recorded with it: **§8's AV-ownership table is rewritten** to match Q7 (CEC primary; `<Active Source>` the primary ownership witness and the AVR's `SI` the secondary, IP-path one), which PR #502 had left contradicting §13; and **§9's CEC-watchdog stand-down is DONE, not outstanding** — verified `disabled`/`inactive` on htpc-1 read-only, with the `Type=notify`+`WatchdogSec=` design being what retires it rather than merely disabling it | this document §8, §9, §13 Q7; jedwards1230/tv-shell#504; jedwards1230/homelab-ansible#338 |
| 2026-09-09 | **§13 Q1 answered on hardware, and §7's focus premise is half-confirmed.** `shell-v2` ran under the live gamescope session on the box, as an ordinary client beside Moonlight rather than as the session child, so the boot path was never at risk. It maps, self-tags to app id 9001 **before** map (the only way gamescope resolves it, since post-map tagging is non-functional on 3.16.28 — #472), appears in `focusable_apps`, is switchable with `show 9001`, and renders at 3840x2160 with live core state. The drawer maps as a separate toplevel (`STEAM_OVERLAY` + `STEAM_INPUT_FOCUS`, no `STEAM_GAME`) and **gamescope grants it the keyboard** while the base keeps `GAMESCOPE_FOCUSED_WINDOW` — so the compositor-decides-focus premise holds. Two defects found by the same run: a base surface with no explicit size got Qt's 160x160 default and was upscaled to fill the display (fixed, #479), and **closing the overlay returns focus to nothing and freezes the display on its last frame** (#481). The pad path remains unexercised — keys were XTEST-synthesised and the input core is default-off | this document, #479, #481 |
