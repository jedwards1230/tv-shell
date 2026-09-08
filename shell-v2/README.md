# shell-v2 — the tv-shell v2 shell

Built beside v1's `shell/`, wired into no session. It began as a spike proving
three things under gamescope: **it maps, it self-tags its X11 atoms before it
maps, and it takes controller focus.** All three held on real hardware
(htpc-1, 2026-09-08), so what it renders is now a real home screen rather than a
placeholder grid.

Why the tagging shim has to exist at all is measured, not assumed: gamescope
resolves a window's app id at **creation** and never re-reads it, so a property
arriving after the map has missed the decision (bench 2026-09-06 on the pinned
3.16.28, plus a live control on 2026-09-07 — `../docs/V2_SHELL.md` §8a).

Full rationale, the two repo rules it reverses, the mutation record, and (at
equal length) what it does **not** prove: [`../docs/V2_SHELL.md`](../docs/V2_SHELL.md).
The compositor contract it implements: [`../docs/V2_DESIGN.md`](../docs/V2_DESIGN.md) §5, §7, §13 Q1.

## Layout

```
CMakeLists.txt          The build step v1 deliberately lacks. ONE QML module, URI TvShell.
src/
  surfacetags.{h,cpp}   PURE role -> STEAM_* atoms. No Qt GUI, no X in its link line.
  paths.{h,cpp}         PURE path resolution: the core socket, the catalog file.
  x11tagger.{h,cpp}     The ONLY place the shell speaks X (mirrors core/src/atoms.rs).
  surface.{h,cpp}       Surface: a toplevel that declares its ROLE and tags before map.
  coreclient.{h,cpp}    The ONLY channel to tv-shell-core. One socket; no Process, ever.
  shellconfig.{h,cpp}   Reads shell.json and hands QML its text. I/O only; no parsing.
  main.cpp              Entry point; warns loudly on a non-xcb platform plugin.
qml/TvShell/            The single QML module — no qmldir, no relative-dir imports.
  Main.qml              Composition root: three toplevels, one core connection.
  HomeScreen.qml        The home screen. Renders the model; owns none of it.
  DrawerScreen.qml      The overlay toplevel's content.
  Rail.qml              A row of cards that keeps the focused one in view.
  Card.qml              The shell's ONE focusable thing. One focus ring, everywhere.
  Tokens.qml            The design system: colour, type, space, duration. Singleton.
  FocusRouter.qml       Owns currentId; every decision delegates to focusGraph.js.
  FocusSlot.qml         One cell: declares WHERE it sits, never who its neighbours are.
  focusGraph.js         Pure: neighbour / rehome / initial / problems.
  homeModel.js          Pure: (catalog, core snapshot) -> rails. The whole home screen.
  catalog.js            Pure: shell.json text -> entries + problems.
  viewport.js           Pure: scroll-into-view offset, and the UI scale.
tests/                  Four lanes — see below.
```

## Five things to know before editing

1. **`Surface` tags before map, and Qt gives no virtual hook to do it.**
   `QWindow::setVisible` is not virtual in Qt 6 and neither is `create()`. The
   ordering is enforced by redeclaring the `visible` property, hiding the base
   `setVisible`, **deleting** `show()`/`showNormal()`/`showFullScreen()`/
   `showMaximized()`, and deferring visibility to `componentComplete()`. If you
   find yourself adding a way to show a window, you are about to map an untagged
   one. Read `src/surface.h` first.

2. **Nothing sets an atom directly.** A caller sets `role`; the role decides the
   atoms. That is what makes an overlay drawn inside the base window
   unrepresentable rather than merely discouraged.

3. **Focus is computed, never wired.** A `FocusSlot` declares `row`, `column` and
   `slotEnabled`. It does not name a neighbour, and neither does anything else.
   Disabling a slot cannot strand focus: traversal skips empty rows, and the
   router re-homes off a cell that stops being focusable. The cell set is judged
   once per event-loop turn (R6), so a screen rebuilding its delegates does not
   throw focus away and re-place it. Put new decision logic in `focusGraph.js`
   (pure, headlessly tested), not in a binding.

4. **The core owns state; the shell renders it.** The home screen is a pure
   function of two inputs — `shell.json` and one `screen-state` snapshot — in
   `homeModel.js`. There is no cache, no "what did I launch" bookkeeping, and no
   second opinion about what is running. If a screen needs something the core
   does not publish, that is a finding to report, not a licence to shell out:
   there are **zero** `Process` sites in this tree, against 50 in v1's `shell/`.

5. **There is no polling, because there is nothing to subscribe to.** The core
   ships no event stream yet (`core/src/protocol.rs` says so in as many words),
   so the shell asks for a snapshot at the three moments that can have changed it
   — the connection coming up, one of its own commands completing, the drawer
   closing — and is otherwise silent. The honest consequence: an app that exits on
   its own leaves a stale "Running" badge until the next of those. The fix belongs
   in the core.

## Configuration

`~/.config/tv-shell/shell.json` — display metadata for launchable apps. See
[`../docs/V2_SHELL_CATALOG.md`](../docs/V2_SHELL_CATALOG.md) and
`../config/shell.json.example`. Launch mechanics stay in the core's `core.toml`;
this file never repeats them.

The core socket is `$TV_SHELL_CORE_SOCK`, else `/run/user/<uid>/tv-shell-core.sock`
— the same rule as `core/src/config.rs`, with a test that spells the default out
so a rename on either side fails loudly.

## Build and test

```bash
cmake -S shell-v2 -B build -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build build
cmake --build build --target all_qmllint
cmake --build build --target qmllint_strict
ctest --test-dir build --output-on-failure
```

| Lane | Needs | Asserts |
|---|---|---|
| `surfacetags` | nothing | the role → atoms mapping, including the two negative rules |
| `coreclient` | nothing | path resolution, and K1–K4 against a real socket and the real framing |
| `qml` | nothing (offscreen) | the four pure modules directly, plus a real HomeScreen over a real router |
| `premap` | a real X server | `PropertyNotify` before `MapNotify`, per role |

`premap` is opt-in behind `TV_SHELL_TEST_XVFB`, read at **configure** time:

```bash
Xvfb :99 -screen 0 1280x800x24 &
TV_SHELL_TEST_XVFB=:99 cmake -S shell-v2 -B build -G Ninja
cmake --build build && ctest --test-dir build --output-on-failure
```

Without it `ctest` reports three lanes; with it, four. CI sets it.

**Run `qmllint_strict`, not just `all_qmllint`.** The generated target runs
qmllint with its defaults, and the defaults let a reference to a member that does
not exist through silently — measured, not assumed. `qmllint_strict` re-runs the
same response file with `missing-property` and `unqualified` promoted to errors.
This matters more here than in most Qt projects: there is no screenshot path on a
v2 session, so a typo'd binding is a property that is simply never set, on a
screen nobody can look at.

## Running it

```bash
QT_QPA_PLATFORM=xcb ./build/tv-shell-v2
```

Arrows move focus, **Enter** activates a card, **Menu** opens the overlay drawer.
With no `shell.json` and no core you get the empty state, which is itself
focusable — the home screen always has somewhere for focus to be.

On a Wayland platform plugin it starts, warns, and maps untagged — under gamescope
that means it is never a focus candidate, which looks like a black screen. The
warning is the diagnosis.
