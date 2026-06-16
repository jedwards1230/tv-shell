import QtQuick
import QtQuick.Layouts
import "lib"

// Widgets settings page — standardized per-widget configuration (#249). Each
// home widget is one bordered **card** grouping its controls together: a title
// row (name + Enabled toggle), a description, and — only when enabled — a Size
// selector. Cards are visually distinct (surface fill + border + consistent
// internal padding) so the three widgets read as three groups, not a flat list.
// Enable/size persist via SettingsStore; the home screen reads them via Theme.
FocusScope {
    id: root
    implicitHeight: mainCol.implicitHeight + 2 * Theme.padding

    function focusFirst() {
        moonlightToggle.forceActiveFocus();
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

    readonly property int _cardPad: Units.spacingLG

    ColumnLayout {
        id: mainCol
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: Units.spacingLG

        // ===== Moonlight =====
        Rectangle {
            Layout.fillWidth: true
            radius: 16
            color: Theme.surface
            border.width: 1
            border.color: Theme.surfaceBorder
            implicitHeight: moonlightCol.implicitHeight + root._cardPad * 2

            ColumnLayout {
                id: moonlightCol
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: root._cardPad
                spacing: Units.spacingMD

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Units.spacingMD

                    Text {
                        text: "Moonlight"
                        font.pixelSize: Theme.fontTitle
                        font.bold: true
                        color: Theme.textPrimary
                        Layout.alignment: Qt.AlignVCenter
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                    FocusButton {
                        id: moonlightToggle
                        Layout.alignment: Qt.AlignVCenter
                        text: Theme.widgetMoonlightEnabled ? "Enabled" : "Disabled"
                        fillActive: Theme.widgetMoonlightEnabled
                        fillColor: Theme.sidebarActive
                        onActivated: SettingsStore.setWidgetMoonlightEnabled(!Theme.widgetMoonlightEnabled)
                        KeyNavigation.down: Theme.widgetMoonlightEnabled ? moonlightSize : nowPlayingToggle
                    }
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Your Moonlight game-streaming servers — pick one to start streaming. Small = an icon-only online rail; Medium = cards with the server name. The full per-host app list still lives in All Apps."
                    font.pixelSize: Theme.fontCaption
                    color: Theme.textMuted
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: Theme.widgetMoonlightEnabled
                    spacing: Units.spacingLG

                    Text {
                        text: "Size"
                        font.pixelSize: Theme.fontBody
                        color: Theme.textSecondary
                        Layout.alignment: Qt.AlignVCenter
                    }
                    SettingsButtonGroup {
                        id: moonlightSize
                        Layout.fillWidth: false
                        Layout.alignment: Qt.AlignVCenter
                        options: root._sizeOptions
                        isCurrentOption: function (opt) {
                            return opt.value === Theme.widgetMoonlightSize;
                        }
                        onValueSelected: opt => SettingsStore.setWidgetMoonlightSize(opt.value)
                        KeyNavigation.up: moonlightToggle
                        KeyNavigation.down: nowPlayingToggle
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                }
            }
        }

        // ===== Now Playing =====
        Rectangle {
            Layout.fillWidth: true
            radius: 16
            color: Theme.surface
            border.width: 1
            border.color: Theme.surfaceBorder
            implicitHeight: nowPlayingCol.implicitHeight + root._cardPad * 2

            ColumnLayout {
                id: nowPlayingCol
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: root._cardPad
                spacing: Units.spacingMD

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Units.spacingMD

                    Text {
                        text: "Now Playing"
                        font.pixelSize: Theme.fontTitle
                        font.bold: true
                        color: Theme.textPrimary
                        Layout.alignment: Qt.AlignVCenter
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                    FocusButton {
                        id: nowPlayingToggle
                        Layout.alignment: Qt.AlignVCenter
                        text: Theme.widgetSpotifyEnabled ? "Enabled" : "Disabled"
                        fillActive: Theme.widgetSpotifyEnabled
                        fillColor: Theme.sidebarActive
                        onActivated: SettingsStore.setWidgetSpotifyEnabled(!Theme.widgetSpotifyEnabled)
                        KeyNavigation.up: Theme.widgetMoonlightEnabled ? moonlightSize : moonlightToggle
                        KeyNavigation.down: Theme.widgetSpotifyEnabled ? nowPlayingSize : plexToggle
                    }
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Cover art, track info, and transport controls for the active media player. Small is a slim strip; Medium is a taller card with a progress bar. When off, the player appears in the Recent row instead."
                    font.pixelSize: Theme.fontCaption
                    color: Theme.textMuted
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: Theme.widgetSpotifyEnabled
                    spacing: Units.spacingLG

                    Text {
                        text: "Size"
                        font.pixelSize: Theme.fontBody
                        color: Theme.textSecondary
                        Layout.alignment: Qt.AlignVCenter
                    }
                    SettingsButtonGroup {
                        id: nowPlayingSize
                        Layout.fillWidth: false
                        Layout.alignment: Qt.AlignVCenter
                        options: root._sizeOptions
                        isCurrentOption: function (opt) {
                            return opt.value === Theme.widgetSpotifySize;
                        }
                        onValueSelected: opt => SettingsStore.setWidgetSpotifySize(opt.value)
                        KeyNavigation.up: nowPlayingToggle
                        KeyNavigation.down: plexToggle
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                }
            }
        }

        // ===== Plex =====
        Rectangle {
            Layout.fillWidth: true
            radius: 16
            color: Theme.surface
            border.width: 1
            border.color: Theme.surfaceBorder
            implicitHeight: plexCol.implicitHeight + root._cardPad * 2

            ColumnLayout {
                id: plexCol
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: root._cardPad
                spacing: Units.spacingMD

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Units.spacingMD

                    Text {
                        text: "Plex"
                        font.pixelSize: Theme.fontTitle
                        font.bold: true
                        color: Theme.textPrimary
                        Layout.alignment: Qt.AlignVCenter
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                    FocusButton {
                        id: plexToggle
                        Layout.alignment: Qt.AlignVCenter
                        text: Theme.widgetPlexEnabled ? "Enabled" : "Disabled"
                        fillActive: Theme.widgetPlexEnabled
                        fillColor: Theme.sidebarActive
                        onActivated: SettingsStore.setWidgetPlexEnabled(!Theme.widgetPlexEnabled)
                        KeyNavigation.up: Theme.widgetSpotifyEnabled ? nowPlayingSize : nowPlayingToggle
                        KeyNavigation.down: Theme.widgetPlexEnabled ? plexSize : recentToggle
                    }
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Up Next (continue watching) and Recently Added in one row — flip between them on the row itself. Small = a poster-only rail; Medium = posters with titles and resume bars."
                    font.pixelSize: Theme.fontCaption
                    color: Theme.textMuted
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: Theme.widgetPlexEnabled
                    spacing: Units.spacingLG

                    Text {
                        text: "Size"
                        font.pixelSize: Theme.fontBody
                        color: Theme.textSecondary
                        Layout.alignment: Qt.AlignVCenter
                    }
                    SettingsButtonGroup {
                        id: plexSize
                        Layout.fillWidth: false
                        Layout.alignment: Qt.AlignVCenter
                        options: root._sizeOptions
                        isCurrentOption: function (opt) {
                            return opt.value === Theme.widgetPlexSize;
                        }
                        onValueSelected: opt => SettingsStore.setWidgetPlexSize(opt.value)
                        KeyNavigation.up: plexToggle
                        KeyNavigation.down: recentToggle
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                }
            }
        }

        // ===== Recent (apps) =====
        Rectangle {
            Layout.fillWidth: true
            radius: 16
            color: Theme.surface
            border.width: 1
            border.color: Theme.surfaceBorder
            implicitHeight: recentCol.implicitHeight + root._cardPad * 2

            ColumnLayout {
                id: recentCol
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: root._cardPad
                spacing: Units.spacingMD

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Units.spacingMD

                    Text {
                        text: "Recent"
                        font.pixelSize: Theme.fontTitle
                        font.bold: true
                        color: Theme.textPrimary
                        Layout.alignment: Qt.AlignVCenter
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                    FocusButton {
                        id: recentToggle
                        Layout.alignment: Qt.AlignVCenter
                        text: Theme.widgetRecentEnabled ? "Enabled" : "Disabled"
                        fillActive: Theme.widgetRecentEnabled
                        fillColor: Theme.sidebarActive
                        onActivated: SettingsStore.setWidgetRecentEnabled(!Theme.widgetRecentEnabled)
                        KeyNavigation.up: Theme.widgetPlexEnabled ? plexSize : plexToggle
                        KeyNavigation.down: Theme.widgetRecentEnabled ? recentSize : recentToggle
                    }
                }

                Text {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Running and recently-launched apps. Small = icon-only tiles; Medium = icon + name cards."
                    font.pixelSize: Theme.fontCaption
                    color: Theme.textMuted
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: Theme.widgetRecentEnabled
                    spacing: Units.spacingLG

                    Text {
                        text: "Size"
                        font.pixelSize: Theme.fontBody
                        color: Theme.textSecondary
                        Layout.alignment: Qt.AlignVCenter
                    }
                    SettingsButtonGroup {
                        id: recentSize
                        Layout.fillWidth: false
                        Layout.alignment: Qt.AlignVCenter
                        options: root._sizeOptions
                        isCurrentOption: function (opt) {
                            return opt.value === Theme.widgetRecentSize;
                        }
                        onValueSelected: opt => SettingsStore.setWidgetRecentSize(opt.value)
                        KeyNavigation.up: recentToggle
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                }
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
