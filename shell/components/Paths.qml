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

    // Per-user tv-shell config dir (~/.config/tv-shell by default).
    readonly property string tvShellConfigDir: configDir + "/tv-shell"

    // Streaming targets file. Resolution order:
    //   1. $TV_SHELL_TARGETS (set by tv-shell-session.sh / a dev override),
    //      falling back to the legacy $GAME_SHELL_TARGETS via Brand.env
    //   2. <tvShellConfigDir>/targets.json
    readonly property string targetsPath: {
        let override = Brand.env("TARGETS");
        if (override && override !== "")
            return override;
        return tvShellConfigDir + "/targets.json";
    }

    // "End game session" command, run on the daemon's `combo:end-session` (the
    // controller end-session combo). The script is supplied by the deployment
    // environment, not the prefix-agnostic shell tree, so its location varies.
    // Resolution order:
    //   1. $TV_SHELL_END_SESSION (deployment override), falling back to the
    //      legacy $GAME_SHELL_END_SESSION via Brand.env
    //   2. /usr/local/bin/end-game-session (documented last-ditch fallback,
    //      matching the TV_SHELL_DIR fallback idiom in tv-shell-session.sh)
    readonly property string endSessionCmd: {
        let override = Brand.env("END_SESSION");
        if (override && override !== "")
            return override;
        return "/usr/local/bin/end-game-session";
    }
}
