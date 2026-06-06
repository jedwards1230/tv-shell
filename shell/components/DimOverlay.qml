import QtQuick

// DimOverlay — OLED burn-in protection (#143).
//
// A dark semi-transparent rectangle that fades in after a short inactivity
// period and clears instantly on ANY user activity. The dim is partial (not
// a full blackout) so the display is protected without disrupting a visible
// stream or running app.
//
// Placement: instantiated once per PanelWindow in shell.qml. Wire activity
// via the `resetDimTimer()` function, which is called whenever the parent
// shell emits a `userActivityDetected` signal (or any Wayland Keys event).
//
// Settings:  SettingsStore.autoDimEnabled / SettingsStore.autoDimDelayMinutes
// Dim level: 0.85 opacity black overlay — OLED-safe and still readable.

Item {
    id: root
    anchors.fill: parent

    // Whether the overlay is currently dimmed (read-only for callers).
    readonly property bool dimmed: dimRect.opacity > 0.0

    // Call from any activity source (userActivity signal, controllerWake, etc.)
    // to clear the dim and restart the inactivity countdown.
    function resetDimTimer() {
        if (SettingsStore.autoDimEnabled)
            dimTimer.restart();
        // Instantly clear the dim on any input (no fade delay on recovery).
        if (dimRect.opacity > 0.0)
            _clearDim();
    }

    // Fade-in animation applied when the inactivity timer fires.
    NumberAnimation {
        id: fadeInAnim
        target: dimRect
        property: "opacity"
        to: 0.85
        duration: 1500
        easing.type: Easing.InQuad
    }

    // Instant clear — stops any in-progress fade and snaps opacity to zero.
    function _clearDim() {
        fadeInAnim.stop();
        dimRect.opacity = 0.0;
    }

    // React to settings changes (enable/disable or delay update).
    Connections {
        target: SettingsStore
        function onAutoDimEnabledChanged() {
            if (SettingsStore.autoDimEnabled) {
                dimTimer.restart();
            } else {
                dimTimer.stop();
                root._clearDim();
            }
        }
        function onAutoDimDelayMinutesChanged() {
            if (SettingsStore.autoDimEnabled)
                dimTimer.restart();
        }
    }

    Component.onCompleted: {
        if (SettingsStore.autoDimEnabled)
            dimTimer.restart();
    }

    // Inactivity countdown — fires once when the idle delay elapses.
    Timer {
        id: dimTimer
        interval: SettingsStore.autoDimDelayMinutes * 60000
        running: SettingsStore.autoDimEnabled
        repeat: false
        onTriggered: {
            if (SettingsStore.autoDimEnabled)
                fadeInAnim.start();
        }
    }

    // The dim overlay. z:200 ensures it paints above all other shell overlays
    // (the debug overlay uses z:100 in ShellLayout). Pointer presses clear the
    // dim immediately and do NOT consume the event so the underlying UI still
    // responds normally after waking.
    Rectangle {
        id: dimRect
        anchors.fill: parent
        color: "black"
        opacity: 0.0
        z: 200

        MouseArea {
            anchors.fill: parent
            // Clear dim on pointer press, but pass the event through so the
            // underlying interactive element also receives it.
            onPressed: mouse => {
                root.resetDimTimer();
                mouse.accepted = false;
            }
        }
    }
}
