pragma Singleton
import Quickshell.Io
import QtQuick

// Centralized settings I/O for game-shell — the QML-side façade over
// ~/.config/game-shell/settings.json.
//
// IMPORTANT: root MUST be Item (not QtObject). Quickshell 0.3.0 cannot host
// Process/Timer children inside a QtObject singleton, and this store needs
// Process children to do socket I/O.
//
// Ownership: the input daemon is the SOLE writer of settings.json. This store
// holds the QML-relevant keys (themeMode, streamingViewMode, controllerDebug,
// rumbleEnabled) and hands them to the daemon via `set-config`; the daemon does
// the
// read-modify-write, preserving foreign keys it owns (notably keyBindings).
// `keyBindings` here is a read-through mirror kept in sync over IPC.
//
// Binding IPC lives here (rather than in InputManager) because keyBindings is a
// setting and this keeps InputManager focused on the core input-grab/combo path.
// IPC protocol: see docs/IPC_PROTOCOL.md
// Commands used: get-config, set-config, get-bindings, set-binding,
//                capture-next, capture-cancel
// Events subscribed: config:changed (live-reload — daemon broadcasts when an
//                    external writer modifies settings.json; QML re-fetches via
//                    get-config so all keys re-apply live without a restart)
// QML-owned display keys: hdrEnabled, nightLightEnabled, nightLightTemp, overscan,
//                         sleepTimerMinutes, wakeOnController, defaultSink,
//                         cecFocusOnStartup, cecFocusOnWake, cecAutoSwitchOnPowerOn,
//                         cecDefaultInput, cecDeviceNames
Item {
    id: store

    // === Persisted settings (QML-owned) ===
    property string themeMode: "dark"             // "auto" | "light" | "dark"
    property string streamingViewMode: "servers"  // "servers" | "apps"
    property bool controllerDebug: false
    property bool rumbleEnabled: true             // gates daemon-fired rumble (#99)
    property bool reduceMotion: false             // suppress animations (#109)
    property real textScale: 1.0                  // font-size multiplier: 1.0/1.15/1.3 (#110)
    property bool hdrEnabled: true               // mirrors config/hyprland.conf cm,hdr default
    property bool nightLightEnabled: false       // drives hyprsunset
    property int nightLightTemp: 4500            // color temperature in Kelvin
    property int overscan: 0                     // safe-area overscan percent (0-10)
    property int sleepTimerMinutes: 0            // 0 = disabled; cycle: 0/5/10/15/30/60
    property bool wakeOnController: true         // declarative preference (no suspend wiring)
    property bool autoDimEnabled: false          // auto-dim OLED protection (#143)
    property int autoDimDelayMinutes: 2          // idle minutes before dimming (#143)
    property string defaultSink: ""              // WirePlumber sink node.name (stable across reboots)
    property bool cecFocusOnStartup: false      // claim active source when daemon starts (default off)
    property bool cecFocusOnWake: true          // claim active source on resume from sleep (default on)
    property bool cecAutoSwitchOnPowerOn: false // switch TV/AVR input when a device powers on (default off, daemon wiring TBD)
    property int cecDefaultInput: -1            // logical address of the preferred default input (-1 = unset; persist-only in Phase 1)
    property var cecDeviceNames: ({})           // local label overrides keyed by logical address, e.g. {"0":"Living Room TV"}

    // === Daemon-owned mirror (authoritative copy lives in the daemon) ===
    property var keyBindings: ({})

    // === Change notification ===
    signal settingsChanged(string key, var value)

    // === Binding IPC signals ===
    signal bindingsReceived(var bindings)
    signal bindingCaptured(string button)
    signal captureCancelled

    // --- Load: read settings via the daemon's `get-config` IPC (no write) ---
    // The daemon is the sole settings.json writer; this asks it for the current
    // document as a compact single-line JSON object.
    SocketClient {
        id: loadProc
        onResponseReceived: line => {
            try {
                var obj = JSON.parse(line);
                if (obj.themeMode === "auto" || obj.themeMode === "light" || obj.themeMode === "dark")
                    store.themeMode = obj.themeMode;
                // Migration: prefer streamingViewMode, fall back to the
                // legacy moonlightViewMode key for existing settings files.
                var viewMode = obj.streamingViewMode !== undefined ? obj.streamingViewMode : obj.moonlightViewMode;
                if (viewMode === "servers" || viewMode === "apps")
                    store.streamingViewMode = viewMode;
                if (typeof obj.controllerDebug === "boolean")
                    store.controllerDebug = obj.controllerDebug;
                if (typeof obj.rumbleEnabled === "boolean")
                    store.rumbleEnabled = obj.rumbleEnabled;
                if (typeof obj.reduceMotion === "boolean")
                    store.reduceMotion = obj.reduceMotion;
                if (typeof obj.textScale === "number")
                    store.textScale = obj.textScale;
                if (typeof obj.hdrEnabled === "boolean")
                    store.hdrEnabled = obj.hdrEnabled;
                if (typeof obj.nightLightEnabled === "boolean")
                    store.nightLightEnabled = obj.nightLightEnabled;
                if (typeof obj.nightLightTemp === "number")
                    store.nightLightTemp = obj.nightLightTemp;
                if (typeof obj.overscan === "number")
                    store.overscan = obj.overscan;
                if (typeof obj.sleepTimerMinutes === "number")
                    store.sleepTimerMinutes = obj.sleepTimerMinutes;
                if (typeof obj.wakeOnController === "boolean")
                    store.wakeOnController = obj.wakeOnController;
                if (typeof obj.autoDimEnabled === "boolean")
                    store.autoDimEnabled = obj.autoDimEnabled;
                if (typeof obj.autoDimDelayMinutes === "number")
                    store.autoDimDelayMinutes = obj.autoDimDelayMinutes;
                if (typeof obj.defaultSink === "string")
                    store.defaultSink = obj.defaultSink;
                if (typeof obj.cecFocusOnStartup === "boolean")
                    store.cecFocusOnStartup = obj.cecFocusOnStartup;
                if (typeof obj.cecFocusOnWake === "boolean")
                    store.cecFocusOnWake = obj.cecFocusOnWake;
                if (typeof obj.cecAutoSwitchOnPowerOn === "boolean")
                    store.cecAutoSwitchOnPowerOn = obj.cecAutoSwitchOnPowerOn;
                if (typeof obj.cecDefaultInput === "number")
                    store.cecDefaultInput = obj.cecDefaultInput;
                if (obj.cecDeviceNames && typeof obj.cecDeviceNames === "object")
                    store.cecDeviceNames = obj.cecDeviceNames;
                if (obj.keyBindings && typeof obj.keyBindings === "object")
                    store.keyBindings = obj.keyBindings;
            } catch (e) {
                console.log("SettingsStore: failed to parse settings:", e);
            }
        }
    }

    // --- Save: hand the QML-owned keys to the daemon via `set-config`. ---
    // The daemon does the read-modify-write, preserving foreign keys (notably
    // daemon-owned keyBindings). The JSON body is built with JSON.stringify and
    // passed as argv to python (not interpolated into the source), so app/enum
    // values can never break the command string. `moonlightViewMode:null` drops
    // the legacy key, matching the previous behavior.
    SocketClient {
        id: saveProc
    }

    function load() {
        loadProc.request("get-config");
    }
    function save() {
        var body = JSON.stringify({
            "themeMode": store.themeMode,
            "streamingViewMode": store.streamingViewMode,
            "controllerDebug": store.controllerDebug,
            "rumbleEnabled": store.rumbleEnabled,
            "reduceMotion": store.reduceMotion,
            "textScale": store.textScale,
            "hdrEnabled": store.hdrEnabled,
            "nightLightEnabled": store.nightLightEnabled,
            "nightLightTemp": store.nightLightTemp,
            "overscan": store.overscan,
            "sleepTimerMinutes": store.sleepTimerMinutes,
            "wakeOnController": store.wakeOnController,
            "autoDimEnabled": store.autoDimEnabled,
            "autoDimDelayMinutes": store.autoDimDelayMinutes,
            "defaultSink": store.defaultSink,
            "cecFocusOnStartup": store.cecFocusOnStartup,
            "cecFocusOnWake": store.cecFocusOnWake,
            "cecAutoSwitchOnPowerOn": store.cecAutoSwitchOnPowerOn,
            "cecDefaultInput": store.cecDefaultInput,
            "cecDeviceNames": store.cecDeviceNames,
            "moonlightViewMode": null
        });
        saveProc.request("set-config", body);
    }

    function setThemeMode(mode) {
        if (mode === "auto" || mode === "light" || mode === "dark") {
            themeMode = mode;
            save();
            settingsChanged("themeMode", mode);
        }
    }

    function setStreamingViewMode(mode) {
        if (mode === "servers" || mode === "apps") {
            streamingViewMode = mode;
            save();
            settingsChanged("streamingViewMode", mode);
        }
    }

    function setControllerDebug(enabled) {
        controllerDebug = enabled;
        save();
        settingsChanged("controllerDebug", enabled);
    }

    function setRumbleEnabled(enabled) {
        rumbleEnabled = enabled;
        save();
        settingsChanged("rumbleEnabled", enabled);
    }

    function setReduceMotion(enabled) {
        reduceMotion = enabled;
        save();
        settingsChanged("reduceMotion", enabled);
    }

    function setTextScale(scale) {
        textScale = scale;
        save();
        settingsChanged("textScale", scale);
    }

    function setHdrEnabled(enabled) {
        hdrEnabled = enabled;
        save();
        settingsChanged("hdrEnabled", enabled);
    }

    function setNightLightEnabled(enabled) {
        nightLightEnabled = enabled;
        save();
        settingsChanged("nightLightEnabled", enabled);
    }

    function setNightLightTemp(temp) {
        nightLightTemp = temp;
        save();
        settingsChanged("nightLightTemp", temp);
    }

    function setOverscan(pct) {
        overscan = pct;
        save();
        settingsChanged("overscan", pct);
    }

    function setSleepTimerMinutes(m) {
        sleepTimerMinutes = m;
        save();
        settingsChanged("sleepTimerMinutes", m);
    }

    function setWakeOnController(enabled) {
        wakeOnController = enabled;
        save();
        settingsChanged("wakeOnController", enabled);
    }

    function setAutoDimEnabled(enabled) {
        autoDimEnabled = enabled;
        save();
        settingsChanged("autoDimEnabled", enabled);
    }

    function setAutoDimDelayMinutes(minutes) {
        autoDimDelayMinutes = minutes;
        save();
        settingsChanged("autoDimDelayMinutes", minutes);
    }

    function setDefaultSink(name) {
        defaultSink = name;
        save();
        settingsChanged("defaultSink", name);
    }

    function setCecFocusOnStartup(enabled) {
        cecFocusOnStartup = enabled;
        save();
        settingsChanged("cecFocusOnStartup", enabled);
    }

    function setCecFocusOnWake(enabled) {
        cecFocusOnWake = enabled;
        save();
        settingsChanged("cecFocusOnWake", enabled);
    }

    function setCecAutoSwitchOnPowerOn(enabled) {
        cecAutoSwitchOnPowerOn = enabled;
        save();
        settingsChanged("cecAutoSwitchOnPowerOn", enabled);
    }

    function setCecDefaultInput(addr) {
        cecDefaultInput = addr;
        save();
        settingsChanged("cecDefaultInput", addr);
    }

    // Set or clear a local name override for a CEC logical address. An empty/blank
    // name removes the override (falls back to the derived friendly name). The
    // value is stored keyed by the address stringified, matching the on-disk shape.
    function setCecDeviceName(addr, name) {
        var key = String(addr);
        var copy = Object.assign({}, cecDeviceNames);
        if (name && name.length > 0)
            copy[key] = name;
        else
            delete copy[key];
        cecDeviceNames = copy;
        save();
        settingsChanged("cecDeviceNames", copy);
    }

    // === Binding IPC (respects GAME_SHELL_SOCK; no hardcoded socket path) ===
    function getBindings() {
        getBindingsProc.request("get-bindings");
    }

    function setBinding(action, button) {
        setBindingProc.request("set-binding " + action + " " + button);
    }

    function captureNext() {
        captureProc.request("capture-next");
    }

    function cancelCapture() {
        cancelCaptureProc.request("capture-cancel");
    }

    // All daemon IPC goes over native Quickshell sockets (SocketClient, #97).
    // The set-config body (a JSON string) is passed as the request body so
    // arbitrary JSON never needs shell/argv quoting. (Phase 8 retired the
    // python3 socket shims.)

    SocketClient {
        id: getBindingsProc
        onResponseReceived: line => {
            try {
                var b = JSON.parse(line);
                store.keyBindings = b;
                store.bindingsReceived(b);
            } catch (e) {
                console.log("SettingsStore: failed to parse bindings:", e);
            }
        }
    }

    SocketClient {
        id: setBindingProc
        // command issued dynamically in setBinding(); re-fetch so the mirror
        // stays current once the daemon acknowledges.
        onResponseReceived: response => store.getBindings()
        onRequestFailed: store.getBindings()
    }

    SocketClient {
        id: captureProc
        onResponseReceived: line => {
            if (line.startsWith("captured:"))
                store.bindingCaptured(line.substring(9));
            else if (line === "timeout" || line === "cancelled")
                store.captureCancelled();
        }
    }

    SocketClient {
        id: cancelCaptureProc
    }

    // --- Live-reload: subscribe to config:changed events from the daemon. ---
    // The daemon inotify-watches settings.json and broadcasts config:changed
    // when an external writer (SSH/Ansible/web UI) modifies it. The daemon
    // suppresses its own set-config/set-binding writes via a self-write
    // generation guard, so this fires only for foreign edits. We re-fetch the
    // full document via get-config (the same path as startup load()), so every
    // QML-owned key re-applies live and keyBindings propagates to consumers.
    SocketClient {
        id: configWatch
        subscribe: true
        onLineReceived: line => {
            if (line === "config:changed")
                store.load();
        }
    }

    // Re-apply persisted default audio sink once at shell startup, after
    // settings have been loaded from the daemon. The AudioSettings page only
    // re-applies when that page is opened; this ensures the correct sink is
    // active even if the user never visits Audio Settings. (#131)
    //
    // The sink name is passed via the GAME_SHELL_SINK environment variable so
    // it cannot inject shell commands regardless of its content. (#131 injection fix)
    Process {
        id: startupSinkApply
        environment: ({
                "GAME_SHELL_SINK": store.defaultSink
            })
        command: ["bash", "-c", "[ -z \"$GAME_SHELL_SINK\" ] && exit 0; " + "if command -v jq >/dev/null 2>&1; then " + "  id=$(pw-dump 2>/dev/null | jq -r --arg n \"$GAME_SHELL_SINK\" " + "    '.[] | select(.info.props[\"node.name\"]==$n) | .id' | head -1); " + "else " + "  id=$(wpctl status 2>/dev/null | grep -F \"$GAME_SHELL_SINK\" | grep -oE '[0-9]+' | head -1); " + "fi; " + "[ -n \"$id\" ] && wpctl set-default \"$id\" || true"]
    }

    onDefaultSinkChanged: {
        if (defaultSink !== "" && !startupSinkApply.running)
            startupSinkApply.running = true;
    }

    Component.onCompleted: {
        load();
        configWatch.start();
    }
}
