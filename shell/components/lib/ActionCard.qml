import QtQuick
import QtQuick.Layouts
import "../"

// Colored-fill action card used in PowerSettings (Sleep/Restart/Shutdown)
// and AVControlSettings (Wake AV/Sleep AV/Switch Input). 7× duplication.
//
// Root MUST be FocusScope so SettingsApp scroll-follow works.
// KeyNavigation wiring stays at the call site — the component cannot
// encapsulate cross-sibling navigation.
//
// Usage:
//   ActionCard {
//       id: sleepScope
//       accentColor: Theme.gold
//       title: "Sleep"
//       subtitle: "Suspend to RAM"
//       activeFocusOnTab: true
//       KeyNavigation.up: above
//       KeyNavigation.down: below
//       onActivated: root.confirmAction = "suspend"
//   }
FocusScope {
    id: root

    property color accentColor: Theme.ember
    property string title: ""
    property string subtitle: ""
    // Border color when not focused. PowerSettings uses Theme.surfaceHover,
    // AVControlSettings uses Theme.surfaceBorder. Defaults to surfaceBorder.
    property color restBorderColor: Theme.surfaceBorder

    signal activated

    implicitWidth: Math.round(Units.gridUnit * 6.3)
    implicitHeight: 120

    activeFocusOnTab: true

    Keys.onReturnPressed: root.activated()
    Keys.onEnterPressed: root.activated()

    Rectangle {
        anchors.fill: parent
        radius: Units.radiusLG
        color: parent.activeFocus ? root.accentColor : Theme.surface
        border.width: parent.activeFocus ? 0 : 2
        border.color: root.restBorderColor

        Behavior on color {
            ColorAnimation {
                duration: 150
            }
        }

        ColumnLayout {
            anchors.centerIn: parent
            spacing: 4

            Text {
                text: root.title
                font.pixelSize: Theme.fontTitle
                font.bold: true
                color: root.activeFocus ? Theme.textOnDark : Theme.textPrimary
                Layout.alignment: Qt.AlignHCenter
            }

            Text {
                text: root.subtitle
                font.pixelSize: Theme.fontSmall
                color: root.activeFocus ? Theme.textOnDarkMuted : Theme.textSecondary
                Layout.alignment: Qt.AlignHCenter
            }
        }

        MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: {
                root.forceActiveFocus();
                root.activated();
            }
        }
    }
}
