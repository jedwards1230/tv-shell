import QtQuick
import Quickshell.Io

// Encapsulates the shell's idle-auto-suspend cluster (#162).
//
// Owns the idle Timer, the logind can-suspend check, and the suspend command.
// shell.qml instantiates this once and calls resetTimer() wherever it
// previously called _resetIdleTimer(). The shell state is injected via the
// `shellState` property so the timer only fires while the shell is idle.
//
// Boot flow:
//   Component.onCompleted → queries power-can-suspend via IPC.
//   Timer fires → if shellState === "idle" && _canSuspend → issues power-suspend.
//
// Activity surface: call resetTimer() on any user interaction (controller,
// keyboard nav, intent signals) that should reset the countdown.
Item {
    id: controller

    // Injected by shell.qml — mirrors root.state ("idle" | "launching" | …).
    property string shellState: "idle"

    // Reset the idle countdown. No-op if the sleep timer is disabled or
    // logind reports suspend is unavailable.
    function resetTimer() {
        var minutes = SettingsStore.sleepTimerMinutes;
        if (minutes > 0 && _canSuspend)
            idleTimer.restart();
    }

    // Whether logind reports suspend is available. Defaults true so the timer
    // fires until told otherwise (fail-safe: suspends rather than never suspends).
    property bool _canSuspend: true

    SocketClient {
        id: suspendCmd
    }

    SocketClient {
        id: canSuspendProc
        onResponseReceived: response => {
            let t = response.trim();
            if (t === "yes")
                controller._canSuspend = true;
            else if (t === "no")
                controller._canSuspend = false;
        }
    }

    Timer {
        id: idleTimer
        interval: SettingsStore.sleepTimerMinutes * 60000
        running: SettingsStore.sleepTimerMinutes > 0 && controller._canSuspend
        repeat: false
        onTriggered: {
            if (controller._canSuspend && controller.shellState === "idle")
                suspendCmd.request("power-suspend");
        }
    }

    // Restart (or stop) the idle timer whenever the sleep-timer setting changes
    // so the new interval takes effect immediately (#162).
    Connections {
        target: SettingsStore
        function onSleepTimerMinutesChanged() {
            if (SettingsStore.sleepTimerMinutes > 0 && controller._canSuspend)
                idleTimer.restart();
            else
                idleTimer.stop();
        }
    }

    Component.onCompleted: {
        // Query logind CanSuspend so the idle timer reflects availability.
        canSuspendProc.request("power-can-suspend");
    }
}
