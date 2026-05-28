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
