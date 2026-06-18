import QtQuick
import QtQuick.Layouts
import "../"

// Metric display card: label, large value, optional subtext, optional
// progress bar. Used in SystemSettings for CPU / Memory / Load Average.
//
// Usage:
//   StatCard {
//       Layout.fillWidth: true
//       label: "CPU Usage"
//       value: root.metricsLoaded ? root.cpuPct.toFixed(1) + "%" : "—"
//       barProgress: root.cpuPct / 100   // 0.0-1.0; -1 to hide the bar
//       barHighColor: root.cpuPct >= 90 ? Theme.crimson : Theme.ember
//   }
Rectangle {
    id: card

    property string label: ""
    property string value: "—"
    // Optional secondary line (e.g. "used / total"). Empty string = hidden.
    property string subtext: ""
    // Progress bar fill 0.0-1.0. Set to -1 (default) to hide the bar.
    property real barProgress: -1
    // Fill color for the progress bar when filled.
    property color barHighColor: Theme.ember

    Layout.fillWidth: true
    Layout.preferredHeight: 170
    radius: Units.radiusMD
    color: Theme.surface
    border.width: 2
    border.color: Theme.surfaceBorder

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 24
        spacing: 12

        Text {
            text: card.label
            font.pixelSize: Theme.fontSmall
            color: Theme.textSecondary
        }

        Text {
            text: card.value
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.textPrimary
        }

        Text {
            visible: card.subtext !== ""
            text: card.subtext
            font.pixelSize: Theme.fontSmall
            color: Theme.textMuted
        }

        Item {
            Layout.fillHeight: true
        }

        // Progress bar — hidden when barProgress < 0
        Rectangle {
            visible: card.barProgress >= 0
            Layout.fillWidth: true
            Layout.preferredHeight: 16
            radius: Units.radiusSM
            color: Theme.surfaceHover

            Rectangle {
                width: parent.width * Math.max(0, Math.min(1, card.barProgress))
                height: parent.height
                radius: Units.radiusSM
                color: card.barHighColor

                Behavior on width {
                    NumberAnimation {
                        duration: 250
                    }
                }
            }
        }
    }
}
