import QtQuick
import QtQuick.Layouts
import "lib"

// Widgets settings page — toggle home-screen widgets (the optional UI cards
// shown above the app/streaming rows) on and off. Toggling only hides the UI
// component; it has no effect on any background/prewarm behaviour.
//
// Spotify (Now Playing): when off, the active media player is no longer
// represented by the widget, so it falls back to the home-screen running row
// (the merged-model filter keys on the widget being visible).
// Plex (On Deck / Recently Added): when off, the two poster rows are hidden.
FocusScope {
    id: root
    implicitHeight: mainCol.implicitHeight + 2 * Theme.padding

    function focusFirst() {
        spotifyScope.forceActiveFocus();
    }

    ColumnLayout {
        id: mainCol
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 32

        // === Now Playing (Spotify) ===
        SectionHeader {
            text: "Now Playing"
        }

        PreferenceRow {
            label: "Show the Now Playing widget"
            description: "Cover art, track info, and transport controls for the active media player. When off, the player appears in the Recent/running row instead."

            FocusButton {
                id: spotifyScope
                KeyNavigation.down: plexScope
                text: Theme.widgetSpotifyEnabled ? "Enabled" : "Disabled"
                fillActive: Theme.widgetSpotifyEnabled
                fillColor: Theme.sidebarActive
                onActivated: Theme.setWidgetSpotifyEnabled(!Theme.widgetSpotifyEnabled)
            }
        }

        // Divider
        Rectangle {
            Layout.fillWidth: true
            height: 1
            color: Theme.surfaceBorder
        }

        // === Plex ===
        SectionHeader {
            text: "Plex"
        }

        PreferenceRow {
            label: "Show the Plex widget"
            description: "On Deck (continue watching) and Recently Added poster rows from your Plex server."

            FocusButton {
                id: plexScope
                KeyNavigation.up: spotifyScope
                text: Theme.widgetPlexEnabled ? "Enabled" : "Disabled"
                fillActive: Theme.widgetPlexEnabled
                fillColor: Theme.sidebarActive
                onActivated: Theme.setWidgetPlexEnabled(!Theme.widgetPlexEnabled)
            }
        }

        Item {
            Layout.fillHeight: true
        }

        HintBar {
            text: "Widgets only change the home screen — nothing is disabled in the background."
        }
    }
}
