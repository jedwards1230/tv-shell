# `shell.json` — the v2 shell's app catalog

Copy `shell.json.example` to `~/.config/tv-shell/shell.json`. Override the path
with `$TV_SHELL_SHELL_JSON`.

**This file is display metadata only.** How to *start* an app — the argv, the
environment, the `env_unset` that decides whether a client maps a window at all —
lives in the core's `[[app]]` table in `core.toml`, and is never duplicated here.
The shell launches by id (`launch <id>`) and the core resolves the rest, so the
command line exists in exactly one place.

The split exists because the core does not publish its `[[app]]` table: there is
no `list-apps` verb in the v2 IPC, so the shell has no way to ask what is
launchable. If one is added, the shell can take ids from the core and this file
becomes purely cosmetic. Until then an app id appears in two files, and they must
agree — an id here that the core does not know will fail at launch, and the toast
will say so.

## Fields

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | The gamescope app id. A whole number, 0 … 4294967295. Must match an `[[app]]` id in `core.toml`. |
| `title` | yes | What the card says. |
| `subtitle` | no | A second line on the card. Omit for none. |
| `accent` | no | A colour for the card's accent stripe. Omit to use the theme's. |

## Rules the shell applies to this file

These are enforced by `shell-v2/qml/TvShell/catalog.js` and each has a test.

- **A missing or empty file is not an error.** You get an empty home screen with
  an empty state, not a warning. A box that has not configured a catalog has not
  made a mistake.
- **A broken file IS reported** — a JSON syntax error, or `apps` not being an
  array, logs a problem and yields an empty catalog. It never takes the shell
  down.
- **One bad row does not discard the others.** A row missing `id` or `title` is
  dropped and logged; the rows around it still appear.
- **A duplicate `id` keeps the first row** and logs the collision.
- **Order is file order.** The Apps rail is this list, top to bottom, left to
  right. Nothing is sorted, so the layout is something you edit.

## What is NOT in this file

- **Running apps.** The Continue rail comes from the core's `screen-state`
  snapshot, never from this file and never from a launch the shell remembers
  making. An app that is running but absent from this file still appears, titled
  by its id — the core is the authority on what exists.
- **The shell itself.** The shell is always a focus candidate, so its own id is
  filtered out of every rail. Listing it here changes nothing.
