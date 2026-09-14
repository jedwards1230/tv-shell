# Issue drafts from the §13 decisions (2026-09-14)

These are ready-to-file drafts for the actionable work falling out of the
[`V2_DESIGN.md`](V2_DESIGN.md) §13 decisions taken on 2026-09-14. They live here because the
session that produced them had no authenticated `gh`; each block is fenced so it copies
cleanly into `gh issue create` or the web form.

Order is rough priority. Draft 1 blocks the most and has been flagged three times without
ever being filed.

> **Note on the session target's missing units.** Draft 2 is about
> `tv-shell-v2-cec.service`, which the v2 session target references and which does not exist.
> It is not the only one. The target references **three** not-found units:
> `tv-shell-v2-cec.service`, `tv-shell-v2-stats.service` and `tv-shell-v2-shell.service`.
> Only the first two have drafts here (`-shell.service` is part of draft 1). Whoever picks
> any of this up should know the full set, so that "unit not found" in the journal is read as
> a known gap rather than a new fault.

---

## 1. The v2 shell has no install path

```
Title: v2 shell has no install path — nothing deploys shell-v2/ to the box
```

```markdown
The v2 shell cannot be run on htpc-1 by any supported route. This is the top blocker for v2:
it is why §13 Q2 (plugin mechanism) is deferred, and it is why v2 UI work has no couch to
land on.

## Current state

- `/opt/tv-shell-v2/bin/` contains only `tv-shell-core` and two gamescope scripts. No shell
  binary, no QML module.
- `scripts/install-v2.sh` never touches `shell-v2/` at all — it is not a broken install step,
  there is no install step.
- The v2 session target references `tv-shell-v2-shell.service`, which does not exist. (It
  also references `tv-shell-v2-cec.service` and `tv-shell-v2-stats.service`, likewise absent
  — see the other drafts.)

## Why this is not just "add it to the rsync"

v1's `shell/` is rsync-deployable because it is QML interpreted by a stock runtime. `shell-v2/`
is a plain Qt Quick **application** with a C++ pre-map tagging shim and its own CMake build
(the 2026-09-07 decision in `docs/V2_DESIGN.md` §14, which records exactly this consequence:
"v2's shell is not rsync-deployable (the deploy story is flagged, not solved)").

So this needs a real decision between:

- a **CMake build on-device**, which puts a toolchain on an appliance, or
- a **shipped artifact** built in CI and installed as a package or tarball.

## Scope

- Pick and implement one of the two above.
- Teach `scripts/install-v2.sh` to install the shell binary and its QML module.
- Add the `tv-shell-v2-shell.service` unit the session target already expects, with the
  `Upholds=` relationship described in `docs/V2_DESIGN.md` §9.
- Verify: select the v2 session, get a shell on screen, and confirm v1 still boots after.

## History

Flagged on 2026-09-09, 2026-09-10 and 2026-09-11 without ever being filed. Filing it is the
point of this issue.
```

---

## 2. `tv-shell-v2-cec.service` — CEC as the primary AV backend

```
Title: Add tv-shell-v2-cec.service — kernel pulse8-cec as the primary AV control backend
```

```markdown
Implements the §13 Q7 decision of 2026-09-14, which **amends** the 2026-09-04 decision that
made IP the AV authority and CEC a passive observer. That ordering is reversed: **CEC is the
primary AV control backend**, and Denon telnet / LG webOS+WoL are demoted to a recovery path.

See `docs/V2_DESIGN.md` §13 Q7 for the full rationale. In short: CEC is vendor-neutral where
IP control is Denon-specific telnet plus LG-specific webOS auth that has broken across
firmware revisions; and every reliability failure on the record (adapter wedging, `cec-health`
reporting health it could not know, Plex grabbing `/dev/ttyACM0`, a watchdog that "recovered"
a daemon three times that was never broken) is a **libcec/watchdog** failure, not a CEC
protocol failure. Kernel `pulse8-cec` plus the kernel CEC API is a much thinner surface.

## Step one: there is no CEC device yet

Measured on htpc-1 2026-09-14:

- `pulse8-cec` is **not loaded**.
- There is **no `/dev/cec*` node at all**.
- The `cec` core module is loaded only because amdgpu pulls it in — nothing is using the
  adapter.
- The Pulse-Eight adapter is sitting as a raw serial device at `/dev/ttyACM0`, held by nobody
  (`fuser` returns no holder).

So the first task is loading `pulse8-cec` to obtain a `/dev/cecN`. There is currently no
kernel CEC device for a daemon to open.

## Scope

- Load `pulse8-cec` persistently (module config + whatever binds the Pulse-Eight adapter away
  from the raw `ttyACM0` path).
- New daemon, its own unit `tv-shell-v2-cec.service` — **not** in the core. The wedge history
  is real and the core is what keeps Moonlight alive; a hung CEC backend must not be able to
  take the compositor's supervisor down with it.
- The daemon owns **power, input switching and volume** over the kernel CEC API.
- `libcec` / `cec-rs` stay dropped.
- **IP recovery path**: keep the existing Denon telnet and LG webOS/WoL implementations, used
  when the CEC bus is unavailable or the adapter has wedged.

## Why the fallback is worth building

It retires jedwards1230/tv-shell#251. Today a true adapter wedge costs a reboot. With an IP
recovery path it costs a degraded mode that is recoverable in place — the television still
responds, through a worse door.
```

---

## 3. Screenshot reachability — expose the capture verb off-box

```
Title: Expose the core's screenshot capture off-box through the panel
```

```markdown
Implements §13 Q13 as answered 2026-09-14: **fix reachability first, fidelity later.**

v1's screen capture was reachable over HTTP and MCP. v2's capture verb is **on-box only**, so
anyone doing v2 UI work who is not physically sitting at the television is working blind.
That is a bigger problem than the question §13 Q13 originally asked (which capture path is
more faithful under HDR).

## Scope

- Keep the existing X root-property capture as the mechanism. It is what gamescope offers —
  gamescope implements no Wayland capture protocol, which is why capture goes through an X
  root property in the first place.
- Expose it through the **panel**, so the image can be retrieved off-box the way v1's could.
- Note the panel's own placement constraint from `docs/V2_DESIGN.md` §12: the panel belongs to
  `default.target`, not the session target, so the recovery/observation surface does not die
  with the thing it observes.

## Explicitly out of scope

HDR capture **fidelity** — `gamescope_control`'s `screen_buffer` type versus a WSI-side
capture — is deferred behind this. A more faithful image nobody can retrieve is worth less
than an imperfect one that reaches the person doing the work.
```

---

## 4. Re-measure Steam Remote Play HDR on current builds

```
Title: Re-measure Remote Play HDR — full Steam client receiver AND standalone Steam Link
```

```markdown
Implements the §13 Q3 reopening of 2026-09-14.

## What the record currently over-claims

`docs/V2_DESIGN.md` §12 and the decision log read as "Remote Play is SDR on Linux by a
Valve-side gate". The actual 2026-09-05 measurement was narrower: the Remote Play **receiver**
(`streaming_client`) declined an HDR swapchain that gamescope was offering it. The WSI layer
logged `server hdr output enabled: true` / `hdr formats exposed to client: true`; every
swapchain the client created came back `VK_FORMAT_B8G8R8A8_UNORM` /
`VK_COLOR_SPACE_SRGB_NONLINEAR_KHR`; `streaming_client.log` said it was "using the sRGB
colorspace" — including under the Deck flags `-steamos3 -steampal -steamdeck -gamepadui`.

That is **one client, on one build, declining one offer**. It is not evidence that
Steam/gamescope HDR is broken on Linux generally: local games under gamescope get HDR fine
(SteamOS and the Steam Deck do exactly this), and htpc-1 itself runs Moonlight at 4K120 HDR10
through the same compositor and the same WSI layer.

## Why re-measure

- The 09-05 run **was already the full Steam client path** — Big Picture's Remote Play uses
  `streaming_client` — so the genuinely untested flavour is **standalone Steam Link**, which is
  still not installed on the box.
- The result is now 9+ days old on a client that ships continuously.
- The test session had **toggled between the `steamdeck_stable` and `publicbeta` betas**, so
  it is not safe to assume the result describes the build a user would get today.

## Scope

Measure **both** receivers on current builds:

1. The full Steam client in Big Picture (`streaming_client`).
2. The standalone Steam Link client (needs installing first; today only the kit's
   "not installed → exit 2" path is exercised).

For each, record: the swapchain format and colorspace the client creates, the WSI layer's
`hdr formats exposed` line, what `streaming_client.log` says about colorspace, **and the beta
channel the client was on** — the last one was left to drift and it should not be again.

Outcome either restores the SDR finding on current builds, or narrows/retires it.
```

---

## 5. Headless CI spike — real gamescope under lavapipe

```
Title: Spike: real gamescope under lavapipe on a hosted runner
```

```markdown
Implements §13 Q10 as answered 2026-09-14: **spike it.**

## The evidence that justifies it

The offline fixtures use a **fake `xprop`**. They therefore test our logic and never test the
compositor's behaviour — which is exactly how **126 assertions passed green while the box was
entirely broken on the 3.16.28 pin**. That is not a coverage gap; it is a class of bug the
current suite structurally cannot see. A real gamescope in CI is the only thing that catches
it.

## Scope

- Stand up gamescope headless on a hosted runner under lavapipe.
- Verify the three unknowns named in §13 Q10: lavapipe acceptance, stats emission, and
  `GAMESCOPE_CREATE_XWAYLAND_SERVER` under headless.
- Drive at least one end-to-end assertion through **real** atoms rather than the fake `xprop`
  — ideally the pre-map tagging contract, since that is the one whose failure mode was
  invisible.

## Known risk, recorded up front

**lavapipe may simply not cooperate on a hosted runner.** That is why this is a spike and not
a task. If it does not work, the deliverable is a documented negative and the fixtures stay
what they are, honestly labelled as logic-only.
```

---

## 6. VRR per-app config plumbing

```
Title: Add [[app]] config plumbing for per-app VRR, with [display].vrr as the default
```

```markdown
Implements §13 Q11 as answered 2026-09-14: **VRR is per-app, defaulting to on.**

## Why per-app

The risk and the benefit are not in the same place. The OLED near-black flicker the ops record
warns about is worst on **static near-black UI** — the shell's drawer and QAM, and video
letterboxing — while VRR earns its keep on content that is neither static nor near-black:
Moonlight and games. So VRR on for streaming clients and games, off for the shell and for
video.

## Scope

- Add a **`[[app]]` config table** to `core.toml` with a per-app VRR field. This does not
  exist yet and is the actual blocker for the decision.
- `[display].vrr` becomes the **default** rather than the policy — per-app entries override it.
- Keep the existing config properties intact: every section `#[serde(default)]` so an existing
  `core.toml` keeps loading, and `deny_unknown_fields` so a typo is refused by name rather
  than silently taking a default.
- Set the shipped config so the shell and video apps are VRR-off and Moonlight/games are
  VRR-on.

Until this lands the effective behaviour is the current global default (on), which is the
measured session.
```

---

## 7. Increase journald retention on htpc-1

```
Title: htpc-1 journald retention is too short to survey stream starts
```

```markdown
Blocks concluding §13 Q9 (cause of the launch-coincident hotplug).

## The problem

The approach to Q9 is to check the **free** discriminator first: v2 has been running with
**zero CEC involvement** — no `/dev/cec*`, `pulse8-cec` unloaded, nothing holding
`/dev/ttyACM0`. Observing a launch-coincident hotplug in that state would exonerate the v1 CEC
lifecycle outright and narrow the field to the AVR or an audio infoframe renegotiation at
stream start.

The check came back **inconclusive**, and the reason is retention rather than the hypothesis.
journald on htpc-1 only reaches back to **2026-09-13 16:54** (43.8 MB on disk), so roughly
**21 hours** could be surveyed rather than the **eight days** v2 has actually been running.
A 21-hour window that happened to contain no stream start proves nothing about eight days that
did.

## Scope

- Raise journald retention on htpc-1 so the window covers ordinary use — `SystemMaxUse=` /
  `MaxRetentionSec=` in `journald.conf`, sized against the observed ~44 MB per day.
- This is an htpc-1 host-config change, so it belongs in the Ansible role, not a hand edit on
  the box.

## Then

Either observe a stream start inside the (now longer) retention window, or deliberately
trigger one and survey it, and close out §13 Q9.
```
