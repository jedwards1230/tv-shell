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
                required property int index
                required property var modelData
                target: modelData
                onActivated: root.streamRequested(modelData)

                focus: index === grid.currentIndex

                MouseArea {
                    anchors.fill: parent
                    onClicked: {
                        grid.currentIndex = parent.index
                        parent.forceActiveFocus()
                    }
                    onDoubleClicked: root.streamRequested(parent.modelData)
                }
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
