# Observability

The `game-shell-input` daemon emits observability in Linux-native, standard,
self-describing formats so **any** consumer can collect it their way. This repo
emits the signal; **collection and forwarding are intentionally out of scope**
(they are deployment-private — point your own node_exporter / Prometheus /
journald pipeline at the contract below).

There are two signals:

1. **Logs** → the systemd **journal** (structured, syslog-priority-mapped), with
   a plain-stdout fallback.
2. **Metrics** → a Prometheus/OpenMetrics **textfile** for the node_exporter
   textfile collector (primary), plus a portable HTTP **`/metrics`** scrape
   endpoint (alternative).

---

## Logs

`init_tracing()` chooses a logging backend at startup:

- **systemd journal** (via `tracing-journald`) when a journal is available —
  structured fields + syslog priority mapping (so `journalctl -p` works).
- **stdout** (the original compact `fmt` layer) otherwise, and always on
  non-Linux.

Selection is automatic but overridable via `[observability]` in
`~/.config/game-shell/config.toml`:

| Config key (`[observability]`) | Values | Effect |
|---|---|---|
| `log_journal` | `true` | Force the journald layer on. |
| | `false` | Force stdout (no journald). |
| | _omitted_ (default) | **Auto**: use journald when `JOURNAL_STREAM` is set (i.e. launched under a systemd unit) and the journal socket is reachable; otherwise stdout. |

The log level/targets remain an env var (NOT a config key), so the standard
`RUST_LOG=… game-shell-input` workflow is unchanged:

| Env var | Values | Effect |
|---|---|---|
| `RUST_LOG` | e.g. `info`, `game_shell_input=debug` | Standard `EnvFilter` syntax. Honoured identically on **both** paths. Default `info`. |

If the journald layer is requested but the journal socket cannot be opened, the
daemon logs a one-line notice to stderr and falls back to stdout — it is never
left without logging.

### Reading logs

When run under the user service (the common deployment):

```bash
journalctl --user -u game-shell-input            # all logs
journalctl --user -u game-shell-input -f         # follow
journalctl --user -u game-shell-input -p warning # warnings and above
journalctl --user -u game-shell-input -o json    # structured fields
```

Raise verbosity by setting the `RUST_LOG` env var (it stays an env var, not a
config key), e.g. `RUST_LOG=game_shell_input=debug game-shell-input`. The
`publish` chokepoint at `debug` is a full event tracer (intents, combos,
`pad:*`, input-mode, controller-wake).

---

## Metrics

All metrics are namespaced `game_shell_` and carry `# HELP`/`# TYPE` lines. The
exposition text is rendered **once** by `metrics::render` and shared between the
textfile writer and `/metrics`, so the two never drift.

> **Resource gauges are a convenience.** `game_shell_cpu_percent`,
> `game_shell_mem_*`, `game_shell_load1`, and `game_shell_temperature_celsius`
> are reused from the daemon's existing sys-metrics reader. If a **node_exporter
> is present on the host, prefer its** `node_cpu_*` / `node_memory_*` /
> `node_hwmon_temp_*` — they are more complete and authoritative. The genuinely
> valuable, daemon-specific signal is the **counters** below, which node_exporter
> cannot provide.

### Metric catalogue

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `game_shell_build_info` | gauge | `sha`, `branch`, `version` | Currently deployed revision. Standard info-metric: value is always `1`, identity is in the labels. Resolved **live on each render** from the same provenance as the `/screenshot` `X-GameShell-*` headers and `/dev/status`, so a `/dev/deploy` HEAD swap shows up on the next render. |
| `game_shell_input_events_total` | counter | — | Raw gamepad evdev events read and processed by the input runtime (hot path). |
| `game_shell_intents_emitted_total` | counter | — | Shell intents broadcast (`intent:<name>`) — IPC, HTTP `/intent/*`, MCP `send_intent`, and gamepad Home-tap/Home-hold all funnel through one chokepoint. |
| `game_shell_transitions_total` | counter | — | Shell↔game presenter transitions (`grab`/`release`/`handoff`). |
| `game_shell_pad_joins_total` | counter | — | Gamepads that joined the fleet (hot-join or initial enumeration). |
| `game_shell_pad_leaves_total` | counter | — | Gamepads that left the fleet (disconnect). |
| `game_shell_shell_restarts_total` | counter | — | Daemon starts observed this boot session (the daemon re-execs on `/dev/restart-daemon` and is otherwise supervised, so this is the input-daemon restart count). |
| `game_shell_input_runtime_up` | gauge | — | Input-runtime liveness: `1` while the supervised input event loop is running, `0` during a respawn gap or after it has panicked past its retry budget (the daemon stays alive; IPC input commands then reply `error:input-runtime-down`). Always emitted. |
| `game_shell_input_runtime_restarts_total` | counter | — | **In-process** input-runtime respawns after a caught panic — the supervisor rebuilds the input event loop (fresh fleet → released grabs) without re-execing the daemon. Distinct from `game_shell_shell_restarts_total` (whole-process starts); a rising value flags a recurring panic in the input path. |
| `game_shell_grab_invariant_violations_total` | counter | — | Detected grab-state drift: a pad's physical `EVIOCGRAB` disagreed with the presenter policy (`should_grab`) after a transition. Should stay `0`; nonzero means the daemon's grab bookkeeping and the kernel diverged. |
| `game_shell_deploy_total` | counter | `outcome` (`ok`\|`error`) | `POST /dev/deploy` attempts via the HTTP bridge, split by success/failure. |
| `game_shell_build_total` | counter | — | `POST /dev/build` attempts via the HTTP bridge. |
| `game_shell_restart_shell_total` | counter | — | `POST /dev/restart-shell` attempts via the HTTP bridge. |
| `game_shell_restart_daemon_total` | counter | — | `POST /dev/restart-daemon` (re-exec) requests via the HTTP bridge. Counted before the process image is replaced; the re-exec'd process starts its own counters at zero. |
| `game_shell_cpu_percent` | gauge | — | Aggregate CPU utilisation 0..=100. _Convenience — prefer node_exporter._ |
| `game_shell_mem_used_bytes` | gauge | — | Used memory in bytes. _Convenience._ |
| `game_shell_mem_total_bytes` | gauge | — | Total memory in bytes. _Convenience._ |
| `game_shell_load1` | gauge | — | 1-minute load average. _Convenience._ |
| `game_shell_temperature_celsius` | gauge | `sensor` | Per-sensor hardware temperature (e.g. `sensor="CPU Tctl"`). _Convenience._ |

### Option A — node_exporter textfile collector (primary)

A background task periodically renders the exposition text and writes it
**atomically** (temp file + `rename(2)`, as the textfile collector requires) to a
`.prom` file.

| Config key (`[observability]`) | Default | Effect |
|---|---|---|
| `metrics_textfile` | _omitted_ → **writer disabled** | Absolute path to the `.prom` file to write (e.g. `/var/lib/node_exporter/textfile/game-shell.prom`). |
| `metrics_interval` | `15` | Render/write interval in seconds. `0` falls back to the default. |

When `metrics_textfile` is omitted, **no file is written** — the textfile path is
opt-in. The `/metrics` HTTP route is unaffected by this setting.

Point node_exporter's textfile collector at the file's **directory** (see
[`examples/README.md`](../examples/README.md)).

### Option B — scrape `/metrics` (portable alternative)

When the HTTP bridge is bound (`[http].bind` in config.toml), it serves:

```
GET /metrics  →  200, Content-Type: text/plain; version=0.0.4; charset=utf-8
```

This route **bypasses the bearer-token auth** (scrapers don't send tokens) and
exposes only aggregate counters + resource gauges (no screen content, no
control). It is always available and cheap. See
[`examples/prometheus-scrape.yaml`](../examples/prometheus-scrape.yaml).

```bash
curl -s http://<host>:<port>/metrics
```

---

## Configuration summary

Everything below is `[observability]` in `~/.config/game-shell/config.toml`,
except `RUST_LOG` which stays a standard env var:

| Setting | Default | Purpose |
|---|---|---|
| `[observability].log_journal` | auto | `true`/`false` to force journald on/off; omitted = auto-detect. |
| `RUST_LOG` (env) | `info` | `EnvFilter` log level/targets (both logging paths). |
| `[observability].metrics_textfile` | omitted (disabled) | Path to the `.prom` textfile-collector output. |
| `[observability].metrics_interval` | `15` | Textfile render/write interval (seconds). |

See [`config/config.toml.example`](../config/config.toml.example) for
copy-runnable defaults and [`examples/`](../examples/) for a starter Grafana
dashboard and a Prometheus scrape snippet.
