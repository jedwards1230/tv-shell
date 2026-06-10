pragma Singleton

import Quickshell
import QtQuick

// Centralized config-path resolution for the QML shell (#145).
//
// Keeps the streaming-targets path in ONE place so the read (MoonlightProvider /
// MoonlightSettings load) and the write (MoonlightSettings tee) can never drift.
// Resolution mirrors the env idiom used in SocketClient.qml / MoonlightConf.qml.
Item {
    id: paths

    // Per-user config dir: $XDG_CONFIG_HOME, else $HOME/.config.
    readonly property string configDir: {
        let xdg = Quickshell.env("XDG_CONFIG_HOME");
        if (xdg && xdg !== "")
            return xdg;
        let home = Quickshell.env("HOME") || "";
        return home + "/.config";
    }

    // Per-user game-shell config dir (~/.config/game-shell by default).
    readonly property string gameShellConfigDir: configDir + "/game-shell"

    // Streaming targets file. Resolution order:
    //   1. $GAME_SHELL_TARGETS (set by game-shell-session.sh / a dev override)
    //   2. <gameShellConfigDir>/targets.json
    readonly property string targetsPath: {
        let override = Quickshell.env("GAME_SHELL_TARGETS");
        if (override && override !== "")
            return override;
        return gameShellConfigDir + "/targets.json";
    }
}
