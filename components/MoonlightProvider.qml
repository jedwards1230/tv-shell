import QtQuick
import Quickshell.Io

// Moonlight streaming backend. Owns targets.json I/O, app discovery via
// `moonlight list`, and the Moonlight CLI argv for launch/quit. Implements the
// TargetProvider contract.
TargetProvider {
    id: provider

    providerId: "moonlight"
    displayName: "Moonlight"

    function loadTargets() {
        loadProc.running = true;
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

    // Streaming targets are loaded from /opt/game-shell/targets.json (single
    // line — SplitParser reads line-by-line).
    Process {
        id: loadProc
        command: ["cat", "/opt/game-shell/targets.json"]
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
