import QtQuick
import QtQuick.Layouts
import "lib"

// Widgets settings page — standardized per-widget configuration (#249). Each
// home widget gets one uniform block: an enable toggle, and (only when enabled)
// a revealed Size selector. Toggling a widget off hides its size options
// entirely. Sizing/enable persist via SettingsStore; the home screen reads them
// through Theme.
FocusScope {
    id: root
    implicitHeight: mainCol.implicitHeight + 2 * Theme.padding

    function focusFirst() {
        nowPlayingToggle.forceActiveFocus();
    }

    readonly property var _sizeOptions: [
        {
            "label": "Small",
            "value": "small"
        },
        {
            "label": "Medium",
            "value": "medium"
        }
    ]

    ColumnLayout {
        id: mainCol
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 28

        // ===== Now Playing =====
        SectionHeader {
            text: "Now Playing"
        }

        PreferenceRow {
            label: "Show the Now Playing widget"
            description: "Cover art, track info, and transport controls for the active media player. When off, the player appears in the Recent row instead."

            FocusButton {
                id: nowPlayingToggle
                text: Theme.widgetSpotifyEnabled ? "Enabled" : "Disabled"
                fillActive: Theme.widgetSpotifyEnabled
                fillColor: Theme.sidebarActive
                onActivated: SettingsStore.setWidgetSpotifyEnabled(!Theme.widgetSpotifyEnabled)
                KeyNavigation.down: Theme.widgetSpotifyEnabled ? nowPlayingSize : plexToggle
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 8
            visible: Theme.widgetSpotifyEnabled

            Text {
                text: "Size"
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textSecondary
            }
            Text {
                text: "Small = a slim strip · Medium = a taller card with a progress bar."
                font.pixelSize: Theme.fontCaption
                color: Theme.textMuted
            }
            SettingsButtonGroup {
                id: nowPlayingSize
                options: root._sizeOptions
                isCurrentOption: function (opt) {
                    return opt.value === Theme.widgetSpotifySize;
                }
                onValueSelected: opt => SettingsStore.setWidgetSpotifySize(opt.value)
                KeyNavigation.up: nowPlayingToggle
                KeyNavigation.down: plexToggle
            }
        }

        Rectangle {
            Layout.fillWidth: true
            height: 1
            color: Theme.surfaceBorder
        }

        // ===== Plex =====
        SectionHeader {
            text: "Plex"
        }

        PreferenceRow {
            label: "Show the Plex widget"
            description: "On Deck (continue watching) and Recently Added poster rows. Recently Added gains category filter chips when more than one media type is present."

            FocusButton {
                id: plexToggle
                text: Theme.widgetPlexEnabled ? "Enabled" : "Disabled"
                fillActive: Theme.widgetPlexEnabled
                fillColor: Theme.sidebarActive
                onActivated: SettingsStore.setWidgetPlexEnabled(!Theme.widgetPlexEnabled)
                KeyNavigation.up: Theme.widgetSpotifyEnabled ? nowPlayingSize : nowPlayingToggle
                KeyNavigation.down: Theme.widgetPlexEnabled ? plexSize : recentToggle
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 8
            visible: Theme.widgetPlexEnabled

            Text {
                text: "Size"
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textSecondary
            }
            Text {
                text: "Poster size for the On Deck and Recently Added rows."
                font.pixelSize: Theme.fontCaption
                color: Theme.textMuted
            }
            SettingsButtonGroup {
                id: plexSize
                options: root._sizeOptions
                isCurrentOption: function (opt) {
                    return opt.value === Theme.widgetPlexSize;
                }
                onValueSelected: opt => SettingsStore.setWidgetPlexSize(opt.value)
                KeyNavigation.up: plexToggle
                KeyNavigation.down: recentToggle
            }
        }

        Rectangle {
            Layout.fillWidth: true
            height: 1
            color: Theme.surfaceBorder
        }

        // ===== Recent (apps) =====
        SectionHeader {
            text: "Recent"
        }

        PreferenceRow {
            label: "Show the Recent widget"
            description: "Running and recently-launched apps as a row of cards."

            FocusButton {
                id: recentToggle
                text: Theme.widgetRecentEnabled ? "Enabled" : "Disabled"
                fillActive: Theme.widgetRecentEnabled
                fillColor: Theme.sidebarActive
                onActivated: SettingsStore.setWidgetRecentEnabled(!Theme.widgetRecentEnabled)
                KeyNavigation.up: Theme.widgetPlexEnabled ? plexSize : plexToggle
                KeyNavigation.down: Theme.widgetRecentEnabled ? recentSize : recentToggle
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 8
            visible: Theme.widgetRecentEnabled

            Text {
                text: "Size"
                font.pixelSize: Theme.fontBody
                font.bold: true
                color: Theme.textSecondary
            }
            Text {
                text: "Card size for the Recent apps row."
                font.pixelSize: Theme.fontCaption
                color: Theme.textMuted
            }
            SettingsButtonGroup {
                id: recentSize
                options: root._sizeOptions
                isCurrentOption: function (opt) {
                    return opt.value === Theme.widgetRecentSize;
                }
                onValueSelected: opt => SettingsStore.setWidgetRecentSize(opt.value)
                KeyNavigation.up: recentToggle
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
