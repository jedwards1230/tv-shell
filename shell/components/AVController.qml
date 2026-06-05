import Quickshell.Io
import QtQuick

// AV power controller for the living-room system (TV + AVR).
//
// Polls AV power status via the daemon's `cec-device <addr>` IPC (logical
// address 5 = AVR / audio-system) every 30 s while the shell is idle, and
// wakes the AV system via `cec-power-on 0` (TV) + `cec-active-source` when
// the shell enters streaming or is asked to wake explicitly.
//
// Public API (unchanged from the prior living-room-cec shell-out version):
//   wake()        — wake if not already on/waking (guarded by cooldown).
//   forceWake()   — unconditional wake (no guard).
//   systemOn      — reflects the AVR power state (bool).
//   shellState    — bound from shell.qml ("idle" enables polling).
Item {
    id: root

    property bool systemOn: false
    property bool waking: false
    property string shellState: ""
    property bool _initialized: false

    function wake() {
        if (waking || avWakeCooldown.running)
            return;
        if (!systemOn)
            waking = true;
        cecPowerOn.request("cec-power-on", "0");
        avWakeCooldown.restart();
    }

    function forceWake() {
        cecPowerOn.request("cec-power-on", "0");
    }

    // --- Status polling (AVR at logical address 5 = AudioSystem) ---

    SocketClient {
        id: cecStatus
        onResponseReceived: line => {
            try {
                var obj = JSON.parse(line);
                if (obj && typeof obj === "object" && "powerStatus" in obj) {
                    root.systemOn = (obj.powerStatus === "on");
                    root._initialized = true;
                }
            } catch (e)
            // parse error or error:* reply — leave systemOn unchanged
            {}
        }
    }

    Timer {
        id: avStatusPoll
        interval: 30000
        running: root.shellState === "idle"
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            cecStatus.request("cec-device", "5");
        }
    }

    Timer {
        id: avWakeCooldown
        interval: 30000
    }

    // --- Wake sequence: power-on TV then set active source ---

    SocketClient {
        id: cecPowerOn
        onResponseReceived: line => {
            if (line === "ok") {
                // Power-on sent — follow up with active-source so the AVR
                // switches to the correct HDMI input.
                cecActiveSource.request("cec-active-source");
            } else {
                root.waking = false;
            }
        }
        onRequestFailed: {
            root.waking = false;
        }
    }

    SocketClient {
        id: cecActiveSource
        onResponseReceived: line => {
            root.waking = false;
            // Re-poll status to confirm wake.
            cecStatus.request("cec-device", "5");
        }
        onRequestFailed: {
            root.waking = false;
        }
    }

    Timer {
        id: avWakeTimeout
        interval: 20000
        running: root.waking
        onTriggered: {
            root.waking = false;
        }
    }

    onSystemOnChanged: {
        if (!root._initialized) {
            root._initialized = true;
            return;
        }
        if (systemOn)
            NotificationManager.info("av", "AV System On");
        else
            NotificationManager.info("av", "AV System Off");
    }
}
