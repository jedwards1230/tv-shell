import QtQuick
import Quickshell.Io

// Moonlight streaming backend. Owns targets.json I/O, app discovery via
// `moonlight list`, and the Moonlight CLI argv for launch/quit. Implements the
// TargetProvider contract.
TargetProvider {
    id: provider

    providerId: "moonlight"
    displayName: "Moonlight"

    // Backend-specific settings UI, loaded on demand by SettingsApp's Loader.
    settingsComponent: Component {
        MoonlightSettings {}
    }

    function loadTargets() {
        loadProc.running = true;
    }

    // Persist a new default `app` for the target matching `host` (the home X-menu
    // "set default" / X action). Writes targets.json via tee + stdin so the value
    // can never break out into a shell command (mirrors MoonlightSettings' write),
    // then reloads so the home widget reflects the change. Local `targets` is
    // updated optimistically so the ● default marker moves on the next open.
    property string _pendingTargetsJson: "[]"
    function setHostApp(host, app) {
        let updated = JSON.parse(JSON.stringify(provider.targets));
        let changed = false;
        for (let i = 0; i < updated.length; i++) {
            if (updated[i].host === host) {
                updated[i].app = app;
                changed = true;
            }
        }
        if (!changed)
            return;
        provider.targets = updated;
        provider._pendingTargetsJson = JSON.stringify(updated);
        saveTargetsProc.running = true;
    }

    Process {
        id: saveTargetsProc
        stdinEnabled: true
        command: ["tee", Paths.targetsPath]
        onStarted: {
            write(provider._pendingTargetsJson);
            stdinEnabled = false; // close stdin -> tee writes the file and exits
        }
        onExited: provider.loadTargets()
    }

    function buildLaunchArgs(target) {
        let args = ["env", "QT_QPA_PLATFORM=wayland", "LIBVA_DRIVER_NAME=radeonsi", "moonlight", "stream", target.host, target.app];
        if (target.resolution === "3840x2160")
            args.push("--4K");
        // Sunshine min_fps_target defaults to 60 when clientRefreshRateX100 is 0.
        // The --fps flag sets the SDP maxFPS but won't raise the server-side floor
        // unless the client also advertises its display refresh rate.
        if (target.fps) {
            args.push("--fps");
            args.push(String(target.fps));
        }
        if (target.hdr)
            args.push("--hdr");
        if (target.codec) {
            args.push("--video-codec");
            args.push(target.codec);
        }
        if (target.bitrate) {
            args.push("--bitrate");
            args.push(String(target.bitrate));
        }
        if (target.audioConfig) {
            args.push("--audio-config");
            args.push(target.audioConfig);
        }
        args.push("--display-mode", "borderless");
        args.push("--no-quit-after");
        args.push("--no-frame-pacing");
        return args;
    }

    function quitArgs(target) {
        return ["moonlight", "quit", target.host];
    }

    // === App discovery (`moonlight list` per host, sequentially) ===
    property int _discoveryIndex: -1

    function discoverApps() {
        if (provider.discovering)
            return;
        provider.discovering = true;
        provider._discoveryIndex = 0;
        provider.hostApps = {};
        provider._discoverNextHost();
    }

    function _discoverNextHost() {
        if (provider._discoveryIndex >= provider.targets.length) {
            provider.discovering = false;
            // Force re-evaluation by reassigning
            provider.hostApps = JSON.parse(JSON.stringify(provider.hostApps));
            return;
        }
        let target = provider.targets[provider._discoveryIndex];
        appDiscovery.currentHost = target.host || "";
        if (appDiscovery.currentHost === "") {
            provider._discoveryIndex++;
            provider._discoverNextHost();
            return;
        }
        // Clear previous results for this host before re-query
        let updated = provider.hostApps;
        updated[appDiscovery.currentHost] = [];
        provider.hostApps = updated;
        appDiscovery.running = true;
    }

    Process {
        id: appDiscovery
        property string currentHost: ""
        command: ["moonlight", "list", currentHost]
        stdout: SplitParser {
            onRead: line => {
                // moonlight list outputs lines like "1. Desktop" or just "Desktop"
                let trimmed = line.trim();
                if (trimmed === "" || trimmed.indexOf("Search") === 0 || trimmed.indexOf("Connect") === 0)
                    return;
                // Strip leading number+dot if present (e.g., "1. Desktop" -> "Desktop")
                let match = trimmed.match(/^\d+\.\s+(.+)/);
                let appName = match ? match[1] : trimmed;
                if (appName === "")
                    return;
                let updated = provider.hostApps;
                if (!updated[appDiscovery.currentHost])
                    updated[appDiscovery.currentHost] = [];
                updated[appDiscovery.currentHost].push(appName);
                provider.hostApps = updated;
            }
        }
        onExited: (exitCode, exitStatus) => {
            if (exitCode !== 0) {
                // Host offline or moonlight list failed — mark empty
                let updated = provider.hostApps;
                updated[appDiscovery.currentHost] = [];
                provider.hostApps = updated;
            }
            // Discover next host
            provider._discoveryIndex++;
            provider._discoverNextHost();
        }
    }

    // Streaming targets are loaded from the resolved targets path (see
    // Paths.targetsPath — $TV_SHELL_TARGETS or ~/.config/tv-shell/targets.json;
    // shared with MoonlightSettings' read/write so they can't drift). Single line —
    // SplitParser reads line-by-line. A missing file yields no lines → targets
    // stays [] (clean no-op, no crash).
    Process {
        id: loadProc
        command: ["cat", Paths.targetsPath]
        stdout: SplitParser {
            onRead: line => {
                try {
                    provider.targets = JSON.parse(line);
                } catch (e) {
                    console.log("MoonlightProvider: failed to parse targets:", e);
                }
            }
        }
    }

    Component.onCompleted: loadTargets()
}
