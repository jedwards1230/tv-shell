# Network Control Surface (HTTP bridge + MCP server)

The daemon exposes its intent/key/screenshot/dev surface over the network two
ways. Both are **opt-in** (a single env var each), share **one bearer token**, and
are thin adapters over the same action logic in `daemon/src/bridge_core.rs`.

| Adapter | Module | Opt-in env | Endpoint |
|---------|--------|------------|----------|
| HTTP/1.1 bridge | `daemon/src/http.rs` | `GAME_SHELL_HTTP_BIND=host:port` | `http://<bind>/...` |
| MCP server (rmcp 1.7.0, streamable-HTTP) | `daemon/src/mcp.rs` | `GAME_SHELL_MCP_BIND=host:port` | `http://<bind>/mcp` |

Relationship to the Unix-socket IPC ([IPC_PROTOCOL.md](IPC_PROTOCOL.md)): the IPC
socket (`0o600`, owner-only) is the shell↔daemon contract. This control surface is
its **network-facing sibling** — same `Control::Intent` / `Control::Key` paths and
the `grim` screenshotter, reachable by off-box clients (an LLM agent, Home
Assistant) when explicitly bound. Unset env → no socket opened, zero exposure.

## Auth model

Both adapters use the **same** vars (`mcp.rs:6`, `http.rs:9`):

| Var | Default | Effect |
|-----|---------|--------|
| `GAME_SHELL_HTTP_AUTH_ENABLED` | enabled | `0`/`false` disables auth (local-only dev) |
| `GAME_SHELL_HTTP_TOKEN` | unset | bearer token; every request needs `Authorization: Bearer <token>` |

- Constant-time compare (`bridge_core::ct_eq_str`).
- Auth enabled + no token → **fail closed** (all 401). Empty token is treated as no token.
- MCP refuses to start on a **non-loopback** bind when dev tools are on AND auth is
  effectively off — that combo is an unauthenticated RCE surface (`mcp.rs:559`).

**Posture**: LAN-only, bind to a trusted interface, keep a token set, leave dev
tools off in production. A wildcard bind (`0.0.0.0`) widens reach — the token is
then the only gate.

## HTTP bridge endpoints (`http.rs`)

| Method | Path | Action |
|--------|------|--------|
| POST | `/intent/<name>` | dispatch intent (`<name>` percent-decoded; see vocab below) |
| POST | `/key/<name>` | synthesize nav key: `up\|down\|left\|right\|select\|back` |
| GET | `/screenshot[.png]` `[?flash=1]` | `grim -` PNG; `flash=1` paints a post-capture vignette. Capture provenance rides in `X-GameShell-{Sha,Branch,Version,Captured-At}` response headers (body stays pure PNG) |
| GET | `/dev/status` | JSON `StatusInfo` blob |
| GET | `/dev/logs` `[?lines=N&filter=str]` | tail `/tmp/qs-log.txt` (lines default 100, max 1000) |
| POST | `/dev/deploy` `[?ref=git-ref]` | git fetch + checkout + reset (ref default `main`) |
| POST | `/dev/build` | run `scripts/build-daemon.sh` + install binary |
| POST | `/dev/restart-shell` | kill + relaunch quickshell, return first WARN/ERROR |
| POST | `/dev/restart-daemon` | re-exec the daemon (picks up a new binary) |

Returns `200 ok`, `400` (`error:` reply), `401`, `404`, `405`, `500` (grim/dev
failure), `503` (daemon unavailable). Hardening: 4 KiB header cap, 5 s header
timeout, 128 concurrent-connection cap (→ 503), 180 s budget for `/dev/*`
subprocesses (auth checked first).

> The HTTP `/dev/*` routes are **always registered** when the bridge is bound —
> they are not behind a separate dev flag (unlike MCP). Gate them by not binding
> the HTTP bridge in production, or bind it to loopback.

## MCP tools (`mcp.rs`)

14 tools over streamable-HTTP at `/mcp`. The 3 dev tools are gated by
`GAME_SHELL_MCP_DEV` (any non-empty value) — when unset they return a clear error
instead of acting (registered unconditionally; rmcp can't yet register
conditionally, `mcp.rs:413`).

| Tool | Params | Annotation | Maps to |
|------|--------|------------|---------|
| `shell_action` | `name` (bare verb only) | write | bare intent from closed vocab |
| `navigate` | `key` (up/down/left/right/select/back) | write | `Control::Key` |
| `open_settings` | `page` (`SettingsPage` enum) | write | `settings:<page>` |
| `open_overlay` | `target` (volume/network/session) | write | `overlay:<target>` |
| `launch_app` | `wm_class` (StartupWMClass) | write | `app:<wm_class>` |
| `list_apps` | — | read-only | XDG `.desktop` scan → `[{name,wm_class,comment}]` |
| `get_ui_state` | — | read-only | Hyprland active window + quickshell focus |
| `take_screenshot` | `flash` (bool) | read-only | `grim` → PNG content + a trailing JSON text block `{captured_at,sha,branch,version}` |
| `get_status` | — | read-only | typed `StatusInfo` JSON (output schema) |
| `get_logs` | `lines` (≤1000), `filter` | read-only | tail `/tmp/qs-log.txt` |
| `restart_shell` | — | destructive | kill + relaunch quickshell |
| `dev_deploy` 🔒 | `git_ref` (default `main`) | destructive | git fetch/checkout/reset |
| `dev_build` 🔒 | — | destructive | build + install binary (~15–60 s) |
| `dev_restart_daemon` 🔒 | — | destructive | re-exec daemon (connection drops) |

🔒 = requires `GAME_SHELL_MCP_DEV`.

**Tool design:**
- `shell_action` accepts only bare verbs from the closed vocabulary (`home`,
  `home-tap`, `home-hold`, `menu`, `settings`, `power`). Deep-links are rejected
  at the MCP layer — use `open_settings` / `open_overlay` / `launch_app` instead.
- `open_settings.page` is a typed `SettingsPage` enum (not a free string).
- `get_ui_state` reports compositor-level window focus (class + title + whether
  quickshell is focused) — NOT QML-internal state. Use `take_screenshot` for
  in-shell view state.
- `take_screenshot` returns capture provenance alongside the frame (HTTP: `X-GameShell-*`
  headers; MCP: a trailing JSON text block) so a caller can tell *which* deployed
  checkout produced the image — latest `main`, a feature branch, or another agent's
  work. It's read live per capture (via `bridge_core::capture_meta`), because a
  `dev_deploy` mutates HEAD under the long-lived daemon without a restart.
- `list_apps` makes `launch_app` discoverable without guessing `wm_class` values.

`GAME_SHELL_MCP_ALLOWED_HOSTS` (comma-separated `host[:port]`) sets the rmcp Host
allowlist (loopback always allowed; a concrete bind IP is auto-added; a wildcard
bind with no override disables Host matching and relies on the token, `mcp.rs:625`).

## Intent vocabulary

Bare: `home`, `home-tap`, `home-hold`, `menu`, `settings`, `power`. Deep-links:
`settings:<page-slug>`, `overlay:<volume|network|session>`, `app:<StartupWMClass>`.
Page slugs: `audio`, `bluetooth`, `network`, `display`, `controllers`,
`keybindings`, `avcontrol`, `widgets`, `accessibility`, `power`, `system`. Validated by
`bridge_core::is_valid_intent` (full vocab in `protocol.rs`; unknown `settings:`
slugs are a graceful QML no-op).

> **Reroute:** `settings:moonlight` and `settings:streaming` are **not** sidebar
> pages — Moonlight server management is demoted under Widgets. Both slugs open
> the **Settings ▸ Widgets ▸ Moonlight** config page directly (server management
> is inlined on it), via the QML `SettingsApp.openSectionById` mapping them to the
> `widgets` section with a pending deep-target. Agents driving the UI should
> expect the Widgets page (not a "Moonlight" sidebar entry) with the
> server-management surface in view.

## `StatusInfo` fields

`sha`, `daemon_pid`, `version`, `shell_running` (`pgrep -x quickshell`),
`wayland_display` (nullable), `hypr_sig_present` (`bridge_core.rs:122`).
