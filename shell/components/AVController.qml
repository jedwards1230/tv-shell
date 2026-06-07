import QtQuick

// AVController — tracks AV system power state and drives wake sequences via the
// daemon's `cec-*` IPC (see docs/IPC_PROTOCOL.md). Replaces the former
// `/usr/local/bin/living-room-cec` Process shell-outs with SocketClient calls so
// the component is generic (no homelab-specific paths) and uses the daemon's
// persistent in-process libcec connection (the reliability fix from #16).
//
// IPC commands used:
//   cec-scan             -> JSON array of {logicalAddress, powerStatus}
//   cec-power-on <addr>  -> ok | error:*
//   cec-active-source    -> ok | error:*
//
// Wake sequence (mirrors the daemon's GAME_SHELL_CEC_LIFECYCLE wake path):
//   1. cec-power-on 5  (AVR = AudioSystem, logical address 5)
//   2. cec-power-on 0  (TV  = Tv,          logical address 0)
//   3. cec-active-source (set this adapter as the active source)
//
// systemOn is derived from the AVR (addr 5) power status in the cec-scan reply.
// When the daemon is built without --features cec (or libcec is absent), every
// cec-* command replies error:* and systemOn stays false — graceful degradation.
Item {
    id: root

    property bool systemOn: false
    property bool waking: false
    property string shellState: ""
    property bool _initialized: false

    function wake() {
        if (waking || _wakeInFlight || avWakeCooldown.running)
            return;
        if (!systemOn)
            waking = true;
        // Kick the three-step wake sequence: power-on AVR → power-on TV → active-source.
        _wakeInFlight = true;
        avWakeOn5.request("cec-power-on 5");
        avWakeCooldown.restart();
    }

    function forceWake() {
        _wakeInFlight = true;
        avWakeOn5.request("cec-power-on 5");
    }

    // Guard: true while a wake sequence is in progress (any of the three steps).
    property bool _wakeInFlight: false

    // --- Status poll (cec-scan) ---

    SocketClient {
        id: avStatusCheck

        onResponseReceived: line => {
            var trimmed = line.trim();
            if (trimmed.length === 0 || trimmed[0] !== "[") {
                // error:* or unexpected — CEC unavailable; leave systemOn as-is.
                root._initialized = true;
                return;
            }
            try {
                var arr = JSON.parse(trimmed);
                // AVR = logical address 5 (AudioSystem).
                var avr = null;
                for (var i = 0; i < arr.length; i++) {
                    if (arr[i].logicalAddress === 5) {
                        avr = arr[i];
                        break;
                    }
                }
                root.systemOn = avr !== null && avr.powerStatus === "on";
                root._initialized = true;
            } catch (e) {
                console.log("AVController: failed to parse cec-scan:", e);
                root._initialized = true;
            }
        }
        onRequestFailed: {
            root._initialized = true;
        }
    }

    Timer {
        id: avStatusPoll
        interval: 30000
        running: root.shellState === "idle"
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            avStatusCheck.request("cec-scan");
        }
    }

    Timer {
        id: avWakeCooldown
        interval: 30000
    }

    // --- Wake sequence: step 1 — power on AVR (addr 5) ---

    SocketClient {
        id: avWakeOn5
        onResponseReceived: response => {
            // Proceed to step 2 regardless of AVR result (TV may already be on).
            avWakeOn0.request("cec-power-on 0");
        }
        onRequestFailed: {
            root._wakeInFlight = false;
            root.waking = false;
        }
    }

    // --- Wake sequence: step 2 — power on TV (addr 0) ---

    SocketClient {
        id: avWakeOn0
        onResponseReceived: response => {
            // Step 3: set active source so the TV switches to this input.
            avWakeActiveSource.request("cec-active-source");
        }
        onRequestFailed: {
            root._wakeInFlight = false;
            root.waking = false;
        }
    }

    // --- Wake sequence: step 3 — set active source ---

    SocketClient {
        id: avWakeActiveSource
        onResponseReceived: response => {
            root._wakeInFlight = false;
            root.waking = false;
            // Re-poll status so systemOn reflects reality after the wake sequence.
            avStatusCheck.request("cec-scan");
        }
        onRequestFailed: {
            root._wakeInFlight = false;
            root.waking = false;
            avStatusCheck.request("cec-scan");
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
