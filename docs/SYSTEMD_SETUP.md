# systemd --user integration

tv-shell runs **both** its input/backend daemon (`tv-shell-input`) **and** the
Quickshell UI (`quickshell -c tv-shell`) as **`systemd --user` services**. This
is an operational wrapper around the same session boot — it does not change *what*
runs, only *how* it's supervised and logged.

## Why

Running each process under `systemd --user` gives, natively and for free:

- **journald log capture with unit metadata** — logs are queryable by unit
  (`journalctl --user -u tv-shell-input`, `-t tv-shell-quickshell`) with
  timestamps, boot IDs, and priority, instead of being absorbed by the display
  manager.
- **cgroup resource accounting** — each unit gets its own cgroup, so
  node_exporter's systemd collector reports per-unit CPU/memory with **zero
  application code**.
- **single-instance + restart semantics** — systemd guarantees one active
  instance per unit and restarts it `on-failure`. This is what actually kills the
  recurring **"3–4 stacked Quickshell instances"** class of bug (#254): a racing
  or re-launched start can't stack a second copy, because the stale instance is
  `stop`ped before a fresh `start` and the daemon's `/dev/restart-shell` prefers
  `systemctl --user restart` over a bare kill+spawn.

Quickshell was previously a bare Hyprland `exec-once` child with **no** supervisor
— no restart-on-crash and no single-instance guarantee. It is now started as
`tv-shell-quickshell.service` (still *by* the compositor's `exec-once`, so
Hyprland owns the window tree, but the process lifecycle is systemd's). Its merged
stdout/stderr are tee'd to both the journal (tagged `tv-shell-quickshell`) and
`/tmp/qs-log.txt` (the dev bridge's log), so existing `journalctl` and
`/dev/logs` queries keep working unchanged.

## What ships

| File | Role |
|------|------|
| `config/tv-shell-input.service` | Daemon user unit (template — `ExecStart` rewritten at install). |
| `config/tv-shell-quickshell.service` | Quickshell UI user unit (installed verbatim; `quickshell` resolves from PATH). |
| `scripts/tv-shell-session.sh` | `systemctl --user start`s the daemon unit (bare-process fallback); `reset-failed`s + `stop`s the Quickshell unit around the session. |
| `config/hyprland.conf` | `exec-once` imports the Wayland session env, then `systemctl --user start`s the Quickshell unit (direct-spawn fallback). |
| `scripts/install.sh` | Installs both units to `~/.config/systemd/user/` + `daemon-reload`. |

## The unit

`config/tv-shell-input.service`:

- `Type=simple`, `ExecStart=<prefix>/bin/tv-shell-input`. The committed copy
  defaults `ExecStart` to `/opt/tv-shell/bin/tv-shell-input`; `install.sh`
  rewrites it to the resolved `--prefix` (the same way it rewrites the session
  `.desktop` `Exec=`).
- **No `EnvironmentFile=`** — per-machine daemon options (HTTP/MCP binds +
  `token_file`, CEC lifecycle, Plex/Steam, observability) live in the typed
  `~/.config/tv-shell/config.toml`, which the daemon reads directly at startup.
  They are deliberately **not** environment variables, which is precisely what
  lets the daemon be configured correctly under this env-less unit (the unit
  inherits none of the session script's environment).
- `Restart=on-failure`, `RestartSec=2`.
- **No `[Install]` section** — deliberate. The session script starts the unit on
  every session, so it must not *also* be enabled into `default.target`; that
  would start a second, environment-less copy at user-manager boot (before
  Hyprland exists), racing the session-owned instance. The session script is the
  single owner of the daemon lifecycle.

The daemon self-discovers everything else: its **install root** from its own
binary path (`current_exe`), its **socket** (`/run/user/$UID/tv-shell-input.sock`
by default — the same path the session script and QML use), and the Wayland /
Hyprland session env (resolved lazily from `$XDG_RUNTIME_DIR`, since the daemon
starts before the compositor). So the unit needs no `Environment=`/`WorkingDirectory=`
wiring.

## The Quickshell unit

`config/tv-shell-quickshell.service`:

- `Type=simple`, `ExecStartPre=-/usr/bin/pkill -x quickshell` (belt-and-braces:
  reap any stray Quickshell before starting; the `-` makes a no-match non-fatal),
  `ExecStart=/bin/bash -o pipefail -c 'quickshell -c tv-shell 2>&1 | tee /tmp/qs-log.txt'`
  (`bash -o pipefail`, not `sh`, so a Quickshell crash propagates through the
  `| tee` pipeline instead of being masked as `tee`'s exit 0 — otherwise
  `Restart=on-failure` would never fire).
  `quickshell` resolves from the user manager's `PATH`, so — unlike the daemon
  unit — there is **no `ExecStart` rewrite** at install; the unit is copied
  verbatim.
- **Dual-sink logging.** `tee` writes the merged output to `/tmp/qs-log.txt` (the
  dev bridge's log — `bridge_core` `get_logs` / `/dev/restart-shell`), and `tee`'s
  stdout flows to the journal under `SyslogIdentifier=tv-shell-quickshell`. `tee`
  truncates the file on each (re)start, matching the old `exec-once` behavior and
  the dev bridge's truncate-on-restart. So both sinks the rest of the system reads
  are preserved.
- `Restart=on-failure`, `RestartSec=2`, `StartLimitBurst=10` /
  `StartLimitIntervalSec=60` — a **higher** burst than the daemon's `3`, because
  this is a kiosk UI on a fast dev loop (deploy → restart-shell → crash-respawn →
  restart again), where a handful of restarts a minute is normal and a budget of 3
  would wedge routine iteration. Belt-and-braces, `/dev/restart-shell`'s systemd
  path also `reset-failed`s the unit before each restart, so a poisoned start
  counter can't leave the shell down.
- **No `[Install]` section** — like the daemon unit. Hyprland's `exec-once` starts
  it each session; enabling it into `default.target` would start a second copy at
  user-manager boot, before a compositor exists.
- **Env is imported, not self-discovered.** Quickshell can't resolve the Wayland
  session env in-process the way the Rust daemon does, so the compositor's
  `exec-once` runs `systemctl --user import-environment WAYLAND_DISPLAY
  HYPRLAND_INSTANCE_SIGNATURE XDG_RUNTIME_DIR` **before** starting the unit. This
  import must happen from inside the running Hyprland session — the session
  wrapper runs before `exec Hyprland`, when those vars don't exist yet, so it
  cannot do the import (it only handles the Quickshell unit's `reset-failed`/`stop`
  lifecycle around the session).

## Session boot flow

`scripts/tv-shell-session.sh` (launched by the display manager):

1. Exports `TV_SHELL_*` session vars (install root, socket, targets path). It no
   longer sources a `daemon.env` — per-machine daemon options are in `config.toml`,
   read by the daemon itself.
2. Starts the daemon:
   - **Preferred:** `systemctl --user start tv-shell-input.service` (after a
     `reset-failed` to clear any stale state).
   - **Fallback:** a bare background process (`"$INPUT_BIN" &`) — the legacy
     path — used when `systemctl --user` is unavailable (no user manager / bus)
     **or** when a dev override `TV_SHELL_INPUT_BIN` is set (the unit's
     `ExecStart` is the installed binary and can't honor an arbitrary override).
   - Also `reset-failed`s `tv-shell-quickshell.service` (best-effort, gated on
     user-systemd availability) so a lingering `StartLimit` failure from a prior
     session doesn't refuse this session's `exec-once` start.
3. `exec`s Hyprland, whose `exec-once` imports the Wayland session env and
   `systemctl --user start`s `tv-shell-quickshell.service` (direct-spawn
   fallback if the user manager or unit is absent).
4. On session exit, the `EXIT` trap `systemctl --user stop`s the daemon unit (or
   `kill`s the bare PID in the fallback path), and also `stop`s the Quickshell
   unit — which runs under the user manager and would otherwise outlive Hyprland
   and race the next session.

The fallback guarantees the session can **never be bricked** by a missing user
manager — a box with no `systemd --user` still boots into the shell exactly as
before, just without the journald/cgroup benefits.

## Inspecting it

```bash
# Daemon unit — status, recent logs, follow
systemctl --user status tv-shell-input
journalctl --user -u tv-shell-input -f

# Quickshell unit — status + logs (SyslogIdentifier keeps the -t query working)
systemctl --user status tv-shell-quickshell
journalctl --user -u tv-shell-quickshell -f    # by unit
journalctl --user -t tv-shell-quickshell -f    # by tag (same stream)

# Confirm exactly one instance of each (single-instance check)
systemctl --user show -p MainPID -p ActiveState tv-shell-input tv-shell-quickshell
pgrep -xc quickshell   # should print 1

# Per-unit cgroup resource accounting (what node_exporter's systemd collector sees)
systemctl --user status tv-shell-input tv-shell-quickshell
systemd-cgtop --user
```

Quickshell's output is still mirrored to `/tmp/qs-log.txt` (the dev bridge's
`/dev/restart-shell` / `/dev/logs` path) in addition to the journal.

## Manual install / enable

`scripts/install.sh` installs and `daemon-reload`s the unit for you. To do it by
hand (e.g. a custom prefix wired up without the installer):

```bash
# Install both units. The daemon unit's ExecStart is rewritten to your prefix
# (awk, not sed, so a prefix with `#`/`&` can't corrupt the unit); the Quickshell
# unit is copied verbatim (quickshell resolves from PATH).
mkdir -p ~/.config/systemd/user
awk -v prefix="$PREFIX" \
    '/^ExecStart=/ { print "ExecStart=" prefix "/bin/tv-shell-input"; next } { print }' \
    "$PREFIX/config/tv-shell-input.service" \
    > ~/.config/systemd/user/tv-shell-input.service
cp "$PREFIX/config/tv-shell-quickshell.service" \
    ~/.config/systemd/user/tv-shell-quickshell.service
systemctl --user daemon-reload
```

You do **not** `systemctl --user enable` it — the session script starts it. If
you want the daemon to also survive logout on a headless/kiosk box, that's
`loginctl enable-linger $USER`, but for the normal "log into the session"
flow the session script's explicit `start`/`stop` is the intended lifecycle.

## Fallback / troubleshooting

- **No journald logs for the daemon?** The bare-process fallback was taken.
  Check `systemctl --user show-environment` works in the session; if not, the
  user manager isn't running. Logs then go wherever the session's stdout goes.
- **Dev override active?** If `TV_SHELL_INPUT_BIN` is set, the session
  intentionally bypasses the unit (bare process) so your override binary runs.
  Unset it to go back through systemd.
- **Two daemons after a crash?** Shouldn't happen — the session does
  `reset-failed` then `start`, and `stop`s on exit. If you started one by hand,
  `systemctl --user stop tv-shell-input` and let the session own it.
- **Stacked Quickshell instances (#254)?** Shouldn't happen under systemd — the
  unit guarantees a single instance and `/dev/restart-shell` prefers
  `systemctl --user restart` over kill+spawn. If `pgrep -xc quickshell` prints >1,
  a non-systemd path stacked them (a manual `quickshell &`, or the exec-once /
  daemon fallback ran because the user manager was unavailable). Recover with
  `systemctl --user restart tv-shell-quickshell` (or `pkill -x quickshell` then
  re-run the exec-once). The daemon bumps `tv_shell_quickshell_multi_instance_total`
  whenever it observes this, so alert on that counter being non-zero.
- **Frequent restarts / unit gives up?** `Restart=on-failure` is rate-limited to
  `StartLimitBurst=3` per `StartLimitIntervalSec=60` — if the daemon hits a
  persistent error (e.g. evdev/uinput permission denied, socket creation failure)
  it restarts at most 3×/60s, then systemd stops trying (no 2s thrash loop).
  Check `journalctl --user -u tv-shell-input` for the root cause; after the
  window elapses, `systemctl --user reset-failed tv-shell-input && systemctl
  --user start tv-shell-input` to retry.
