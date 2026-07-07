pragma Singleton

import Quickshell
import QtQuick

// Central brand identity for the QML shell — the QML mirror of the Rust
// `brand` module (protocol/src/brand.rs). Single source of truth for the
// product slug and the backward-compat env-var shim, so the game-shell →
// tv-shell rebrand's fallbacks live in exactly one place and a box that
// reboots mid-migration keeps working.
QtObject {
    id: brand

    // Current product slug (config dir, session/unit basenames).
    readonly property string slug: "tv-shell"
    // Pre-rename slug, kept for read-fallback + compat.
    readonly property string legacySlug: "game-shell"

    // Current env-var prefix (`TV_SHELL_SOCK`, `TV_SHELL_TARGETS`, …).
    readonly property string envPrefix: "TV_SHELL"
    // Pre-rename env-var prefix, honored as a fallback by env().
    readonly property string legacyEnvPrefix: "GAME_SHELL"

    // Read an env var by its suffix, honoring the legacy prefix as a fallback.
    //
    // env("SOCK") reads TV_SHELL_SOCK, and only if that is unset (or empty)
    // falls back to GAME_SHELL_SOCK. An empty value is treated as unset (matching
    // the Rust `brand::env` "empty == unset" rule) so a stray `TV_SHELL_SOCK=`
    // in the environment does not mask a real legacy value. Returns "" when
    // neither prefix yields a non-empty value.
    function env(suffix) {
        let current = Quickshell.env(envPrefix + "_" + suffix);
        if (current && current !== "")
            return current;
        let legacy = Quickshell.env(legacyEnvPrefix + "_" + suffix);
        if (legacy && legacy !== "")
            return legacy;
        return "";
    }

    // Per-user config base: $XDG_CONFIG_HOME, else $HOME/.config (mirrors the
    // Rust `brand::config_base`).
    readonly property string configBase: {
        let xdg = Quickshell.env("XDG_CONFIG_HOME");
        if (xdg && xdg !== "")
            return xdg;
        let home = Quickshell.env("HOME") || "";
        return home + "/.config";
    }

    // Per-user config directory (~/.config/tv-shell by default).
    //
    // The Rust `brand::config_dir` read-falls-back to ~/.config/game-shell when
    // the new dir is absent, but that needs a synchronous Path::is_dir probe. A
    // plain QtObject singleton can't host the Process/FileView needed to probe
    // the filesystem here, so this defaults to the new path and leaves the
    // migration to env vars (the primary resolution mechanism — the session
    // wrapper exports both TV_SHELL_* and GAME_SHELL_* at the new path) plus the
    // Rust side's read-fallback. This hardcoded value is the secondary path.
    readonly property string configDir: configBase + "/" + slug
}
