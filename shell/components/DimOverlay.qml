import QtQuick
import QtQml

// DimOverlay — OLED burn-in protection (#143).
//
// A dark semi-transparent rectangle that fades in after a short inactivity
// period and clears instantly on ANY user activity. The dim is partial (not
// a full blackout) so the display is protected without disrupting a visible
// stream or running app.
//
// Placement: instantiated once per PanelWindow in shell.qml. Activity is wired
// two ways:
//   1. `resetDimTimer()` — called from shell.qml whenever `userActivityDetected`
//      fires (controller wake, Home/Menu/Settings/Power intents, home-screen
//      navigation). Good for high-level actions.
//   2. While dimmed, an application-wide Shortcut set below catches the FIRST
//      navigation key from any device — even keys consumed by a focused control
//      deep inside a settings page, which never reach the shell's central key
//      observer. This is what makes "wake on any input" work everywhere, not
//      just on the home screen.
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
    //
    // `running` is NOT bound to autoDimEnabled: that binding would re-assert
    // running:true after the one-shot fired (autoDimEnabled stays true), so the
    // timer would restart forever and re-dim immediately. Instead the timer is
    // driven explicitly — restarted on each activity (resetDimTimer / settings
    // changes / startup) and left stopped after it fires, so it dims exactly
    // once per inactivity period and only re-arms on real input.
    Timer {
        id: dimTimer
        interval: SettingsStore.autoDimDelayMinutes * 60000
        repeat: false
        onTriggered: {
            if (SettingsStore.autoDimEnabled)
                fadeInAnim.start();
        }
    }

    // Wake-on-any-input. While dimmed, these application-wide shortcuts fire on
    // the first press of any navigation key regardless of which surface holds
    // focus — so input inside the Settings panel (where the sidebar/page
    // consumes the key before it reaches the shell's central observer) still
    // wakes the screen. Enabled ONLY while dimmed, so they never intercept
    // normal navigation. The activating press is consumed: the first input just
    // wakes; the user navigates with the next press. High-level intents
    // (Home/Menu/Settings/Power) clear the dim via resetDimTimer() separately.
    Instantiator {
        model: ["Up", "Down", "Left", "Right", "Return", "Enter", "Escape", "Tab", "Backspace"]
        delegate: Shortcut {
            sequence: modelData
            context: Qt.ApplicationShortcut
            enabled: root.dimmed
            autoRepeat: false
            onActivated: root.resetDimTimer()
        }
    }

    // The dim overlay. z:200 ensures it paints above all other shell overlays
    // (the debug overlay uses z:100 in ShellLayout). The pointer MouseArea is
    // active only while dimmed, so a press wakes the screen without blocking
    // normal interaction the rest of the time.
    Rectangle {
        id: dimRect
        anchors.fill: parent
        color: "black"
        opacity: 0.0
        z: 200

        MouseArea {
            anchors.fill: parent
            enabled: dimRect.opacity > 0.0
            onPressed: mouse => {
                root.resetDimTimer();
                mouse.accepted = true;
            }
        }
    }
}
