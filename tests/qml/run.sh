#!/usr/bin/env bash
# Headless QML unit tests (QtQuickTest) for the shell's components.
#
# Quickshell-free components can be exercised with the standard Qt qmltestrunner
# under QT_QPA_PLATFORM=offscreen — no compositor, no GPU. The trick is the
# import graph: production components reference singletons (Theme, Units, …) that
# pull in Quickshell, which qmltestrunner can't load. So we assemble a throwaway
# `components` module from:
#   - hand-written STUB singletons (tests/qml/stubs)  — the shim layer
#   - the REAL components under test, copied verbatim  — zero drift
# and point qmltestrunner's import path at it. See tests/qml/README.md.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
build="$here/.build"
shellc="$repo/shell/components"

runner="${QMLTESTRUNNER:-qmltestrunner}"
if ! command -v "$runner" >/dev/null 2>&1; then
    echo "error: '$runner' not found on PATH (install Qt qtdeclarative tools, or set QMLTESTRUNNER)" >&2
    exit 127
fi

# 1. Assemble the stub `components` module (singleton shims + qmldirs).
rm -rf "$build"
mkdir -p "$build/components/lib"
cp "$here/stubs/"*.qml "$build/components/"
cp "$here/stubs/qmldir" "$build/components/qmldir"
cp "$here/stubs/lib/qmldir" "$build/components/lib/qmldir"

# 2. Overlay the REAL components under test (copied, not symlinked — avoids
#    Qt resolving relative imports through the symlink back into shell/).
cp "$shellc/QuickActions.qml" "$build/components/"
cp "$shellc/QuickActionButton.qml" "$build/components/"
cp "$shellc/lib/CountBadge.qml" "$build/components/lib/"
# Widget base + WidgetHost are pure QtQuick (no Quickshell), so they run headless
# against the stub WidgetRegistry + Stub*Widget shims under stubs/lib.
cp "$shellc/lib/Widget.qml" "$build/components/lib/"
cp "$shellc/lib/WidgetHost.qml" "$build/components/lib/"
cp "$here/stubs/lib/"*.qml "$build/components/lib/"

# 3. Run every tst_*.qml in tests/qml headless.
echo "Running QML tests with: $runner (offscreen)"
QT_QPA_PLATFORM=offscreen "$runner" -import "$build" -input "$here"
