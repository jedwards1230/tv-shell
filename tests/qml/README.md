# QML unit tests (QtQuickTest, headless)

Fast, deterministic layout + navigation tests for the shell's QML components.
They run with the stock Qt `qmltestrunner` under `QT_QPA_PLATFORM=offscreen`:
**no Quickshell, no compositor, no GPU.** They answer two questions the
controller-first design depends on:

1. **Is every interactive element reachable by D-pad** and does it clamp at the
   ends? (`Left`/`Right`/`Up`/`Down`/`Escape`)
2. **Does activation (`A`/Return) fire the right behaviour** on each focus stop?

This is layer 2 of the testing strategy (the daemon's Rust unit/integration
tests are layer 1; full-shell pixel screenshots through a headless Wayland
compositor would be layer 3 — not done here).

## Running

```bash
./tests/qml/run.sh
```

Requires Qt's `qmltestrunner` (ships with `qtdeclarative`) on `PATH`. Override
the binary with `QMLTESTRUNNER=/path/to/qmltestrunner`. CI runs this in
`.github/workflows/qml-test.yml` on any change under `shell/**/*.qml` or
`tests/qml/**`.

## How it works — the shim layer

Production components reference singletons (`Theme`, `Units`, `SettingsStore`,
`InputMode`, `IconTheme`, …) that import Quickshell modules (`Quickshell`,
`Quickshell.Io`). `qmltestrunner` has no Quickshell runtime, so it can't load
that import graph directly.

`run.sh` sidesteps this by **assembling a throwaway `components` module** in
`tests/qml/.build/` (gitignored) from two sources:

- **Stub singletons** (`tests/qml/stubs/*.qml`) — plain-QtQuick re-implementations
  of *only* the singleton surface the tested components touch. This is the
  intentional shim: edit these when a tested component starts using a new
  `Theme`/`Units`/etc. property.
- **The real components under test** (e.g. `shell/components/QuickActions.qml`),
  copied verbatim — so the tests exercise production code with **zero drift**.

The assembled dir declares `module components` (and `components.lib`), so the
real files' bare singleton references and `import "lib"` resolve exactly as they
do in the shell — just against stubs.

## Adding a component to the suite

1. Confirm it's Quickshell-free (`import QtQuick` / `import "lib"` only). If it
   imports `Quickshell*` directly it needs decoupling first.
2. In `run.sh`, copy the real file (and any real leaf components it uses) into
   the assembled module.
3. Add it to `tests/qml/stubs/qmldir` (or `stubs/lib/qmldir`).
4. Add any singleton properties it newly relies on to the matching stub.
5. Write `tst_<component>.qml` and run `./tests/qml/run.sh`.

Currently covered: `QuickActions` (+ real `QuickActionButton`, `CountBadge`).
