pragma Singleton
import QtQuick

// Test stub for the production `InputMode` singleton
// (shell/components/InputMode.qml). The real one is pure QtQuick, but it's
// stubbed too so the whole `components` test module is self-contained (no reach
// into shell/, which would drag in Quickshell-importing siblings). See
// tests/qml/README.md.
Item {
    property bool mouseMode: false

    function enterMouseMode() {
        mouseMode = true;
    }

    function exitMouseMode() {
        mouseMode = false;
    }

    // No-op pointer filter — tests drive focus with key events, not the mouse.
    function pointerMoved(gx, gy) {
    }
}
