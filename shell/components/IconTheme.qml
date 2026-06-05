pragma Singleton
import QtQuick
import Quickshell.Io

// Resolves the icon theme base directory once and serves a theme-aware path.
// In dark mode it prefers the dark icon set (light/whitish glyphs); in light
// mode it uses the regular set (dark glyphs). Shared as a singleton so every
// QuickActions row + PowerOverlay reads the same probed value rather than each
// running its own probe (which raced empty for hidden instances).
Item {
    id: root

    property string lightBase: ""   // e.g. /usr/share/icons/breeze
    property string darkBase: ""    // e.g. /usr/share/icons/breeze-dark

    // Active base dir for the current theme. Falls back to whichever set
    // exists if the preferred variant is missing.
    readonly property string base: {
        if (Theme.darkMode)
            return darkBase || lightBase;
        return lightBase || darkBase;
    }

    Process {
        id: probe
        command: ["bash", "-c", "for d in breeze Adwaita hicolor; do [ -d \"/usr/share/icons/$d\" ] && echo \"light:/usr/share/icons/$d\" && break; done; [ -d /usr/share/icons/breeze-dark ] && echo 'dark:/usr/share/icons/breeze-dark'; true"]
        stdout: SplitParser {
            onRead: line => {
                var t = line.trim();
                if (t.startsWith("light:"))
                    root.lightBase = t.substring(6);
                else if (t.startsWith("dark:"))
                    root.darkBase = t.substring(5);
            }
        }
    }

    Component.onCompleted: probe.running = true
}
