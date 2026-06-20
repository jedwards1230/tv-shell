# systemd --user integration

game-shell runs its input/backend daemon (`game-shell-input`) as a
**`systemd --user` service** and routes Quickshell's output into the **journal**.
This is purely an operational wrapper around the same session boot — it does not
change *what* runs, only *how* it's supervised and logged.

## Why

Running the daemon under `systemd --user` gives, natively and for free:

- **journald log capture with unit metadata** — daemon logs are queryable by unit
  (`journalctl --user -u game-shell-input`) with timestamps, boot IDs, and
  priority, instead of being absorbed by the display manager.
- **cgroup resource accounting** — each unit gets its own cgroup, so
  node_exporter's systemd collector reports per-unit CPU/memory with **zero
  application code**.
- **single-instance + restart semantics** — systemd guarantees one active
  instance and restarts the daemon `on-failure`. This also kills the recurring
  "duplicate daemon/quickshell instance" class of bug from a re-launched session
  (a stale instance is `stop`ped before a fresh `start`).

Quickshell itself stays a Hyprland `exec-once` child (Hyprland manages the
compositor's window tree), but its stdout/stderr are piped through `systemd-cat`
so they also land in the journal under a tag.

## What ships

| File | Role |
|------|------|
| `config/game-shell-input.service` | The user unit (template — `ExecStart` rewritten at install). |
| `scripts/game-shell-session.sh` | `systemctl --user start`s the unit (with a bare-process fallback). |
| `config/hyprland.conf` | `exec-once` pipes quickshell through `systemd-cat -t game-shell-quickshell`. |
| `scripts/install.sh` | Installs the unit to `~/.config/systemd/user/` + `daemon-reload`. |

## The unit

`config/game-shell-input.service`:

- `Type=simple`, `ExecStart=<prefix>/bin/game-shell-input`. The committed copy
  defaults `ExecStart` to `/opt/game-shell/bin/game-shell-input`; `install.sh`
  rewrites it to the resolved `--prefix` (the same way it rewrites the session
  `.desktop` `Exec=`).
- `EnvironmentFile=-%h/.config/game-shell/daemon.env` — the leading `-` makes a
  missing file a non-error. (The daemon *also* reads this file itself via
  `session_env::load_daemon_env`, so this is belt-and-suspenders for unit-level
  visibility, e.g. `systemctl --user show-environment`.)
- `Restart=on-failure`, `RestartSec=2`.
- **No `[Install]` section** — deliberate. The session script starts the unit on
  every session, so it must not *also* be enabled into `default.target`; that
  would start a second, environment-less copy at user-manager boot (before
  Hyprland exists), racing the session-owned instance. The session script is the
  single owner of the daemon lifecycle.

The daemon self-discovers everything else: its **install root** from its own
binary path (`current_exe`), its **socket** (`/run/user/$UID/game-shell-input.sock`
by default — the same path the session script and QML use), and the Wayland /
Hyprland session env (resolved lazily from `$XDG_RUNTIME_DIR`, since the daemon
starts before the compositor). So the unit needs no `Environment=`/`WorkingDirectory=`
wiring.

## Session boot flow

`scripts/game-shell-session.sh` (launched by the display manager):

1. Exports `GAME_SHELL_*` session vars and sources `daemon.env`.
2. Starts the daemon:
   - **Preferred:** `systemctl --user start game-shell-input.service` (after a
     `reset-failed` to clear any stale state).
   - **Fallback:** a bare background process (`"$INPUT_BIN" &`) — the legacy
     path — used when `systemctl --user` is unavailable (no user manager / bus)
     **or** when a dev override `GAME_SHELL_INPUT_BIN` is set (the unit's
     `ExecStart` is the installed binary and can't honor an arbitrary override).
3. `exec`s Hyprland, which `exec-once`s quickshell through `systemd-cat`.
4. On session exit, the `EXIT` trap `systemctl --user stop`s the unit (or
   `kill`s the bare PID in the fallback path).

The fallback guarantees the session can **never be bricked** by a missing user
manager — a box with no `systemd --user` still boots into the shell exactly as
before, just without the journald/cgroup benefits.

## Inspecting it

```bash
# Daemon unit — status, recent logs, follow
systemctl --user status game-shell-input
journalctl --user -u game-shell-input -f

# Quickshell output (tagged, not a unit — it's a systemd-cat stream)
journalctl --user -t game-shell-quickshell -f

# Confirm exactly one daemon instance (single-instance check)
systemctl --user show -p MainPID -p ActiveState game-shell-input

# Per-unit cgroup resource accounting (what node_exporter's systemd collector sees)
systemctl --user status game-shell-input   # shows Memory/CPU/Tasks under the cgroup
systemd-cgtop --user
```

Quickshell's output is still mirrored to `/tmp/qs-log.txt` (the dev bridge's
`/dev/restart-shell` / `/dev/logs` path) in addition to the journal.

## Manual install / enable

`scripts/install.sh` installs and `daemon-reload`s the unit for you. To do it by
hand (e.g. a custom prefix wired up without the installer):

```bash
# Install the unit, rewriting ExecStart to your prefix
mkdir -p ~/.config/systemd/user
sed "s#^ExecStart=.*#ExecStart=$PREFIX/bin/game-shell-input#" \
    "$PREFIX/config/game-shell-input.service" \
    > ~/.config/systemd/user/game-shell-input.service
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
- **Dev override active?** If `GAME_SHELL_INPUT_BIN` is set, the session
  intentionally bypasses the unit (bare process) so your override binary runs.
  Unset it to go back through systemd.
- **Two daemons after a crash?** Shouldn't happen — the session does
  `reset-failed` then `start`, and `stop`s on exit. If you started one by hand,
  `systemctl --user stop game-shell-input` and let the session own it.
- **Frequent restarts / unit gives up?** `Restart=on-failure` is rate-limited to
  `StartLimitBurst=3` per `StartLimitIntervalSec=60` — if the daemon hits a
  persistent error (e.g. evdev/uinput permission denied, socket creation failure)
  it restarts at most 3×/60s, then systemd stops trying (no 2s thrash loop).
  Check `journalctl --user -u game-shell-input` for the root cause; after the
  window elapses, `systemctl --user reset-failed game-shell-input && systemctl
  --user start game-shell-input` to retry.
