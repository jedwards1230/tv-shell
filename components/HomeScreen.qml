import QtQuick
import QtQuick.Layouts

Item {
    id: root

    property var targets: []
    property string shellState: "idle"

    signal streamRequested(var target)
    signal settingsRequested()

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: Theme.cardSpacing

        Text {
            text: "Streaming"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.text
        }

        GridView {
            id: grid
            Layout.fillWidth: true
            Layout.fillHeight: true
            cellWidth: Theme.cardWidth + Theme.cardSpacing
            cellHeight: Theme.cardHeight + Theme.cardSpacing
            focus: true
            clip: true

            model: root.targets

            delegate: StreamCard {
                target: modelData
                focus: index === 0
                onActivated: root.streamRequested(modelData)

                KeyNavigation.right: (index + 1 < grid.count) ? grid.itemAtIndex(index + 1) : null
                KeyNavigation.left: (index - 1 >= 0) ? grid.itemAtIndex(index - 1) : null
            }

            Keys.onEscapePressed: root.settingsRequested()
        }

        Text {
            text: "A: Launch  |  B: Settings"
            font.pixelSize: Theme.fontSmall
            color: Theme.textDim
            Layout.alignment: Qt.AlignHCenter
        }
    }
}
