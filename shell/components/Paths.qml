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
    // Requires HOME or XDG_CONFIG_HOME to be set. In a normal systemd session, at
    // least one is guaranteed; if neither is set, this falls back to /.config and
    // will fail at runtime (the mkdir -p / tee write).
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

    // "End game session" command, run on the daemon's `combo:end-session` (the
    // controller end-session combo). The script is supplied by the deployment
    // environment, not the prefix-agnostic shell tree, so its location varies.
    // Resolution order:
    //   1. $GAME_SHELL_END_SESSION (deployment override)
    //   2. /usr/local/bin/end-game-session (documented last-ditch fallback,
    //      matching the GAME_SHELL_DIR fallback idiom in game-shell-session.sh)
    readonly property string endSessionCmd: {
        let override = Quickshell.env("GAME_SHELL_END_SESSION");
        if (override && override !== "")
            return override;
        return "/usr/local/bin/end-game-session";
    }
}
