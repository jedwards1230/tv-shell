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
        spacing: 32

        // Recent row
        Text {
            text: "Recent"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.text
        }

        ListView {
            id: recentRow
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.rowHeight
            orientation: ListView.Horizontal
            spacing: Theme.cardSpacing
            clip: true
            focus: true

            model: root.targets

            delegate: StreamCard {
                required property int index
                required property var modelData
                height: recentRow.height - 20
                width: Theme.cardWidth
                target: modelData
                focus: index === recentRow.currentIndex

                onActivated: root.streamRequested(modelData)

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        recentRow.currentIndex = parent.index
                        parent.forceActiveFocus()
                    }
                    onDoubleClicked: root.streamRequested(parent.modelData)
                }
            }

            Keys.onReturnPressed: {
                if (recentRow.currentItem)
                    root.streamRequested(recentRow.currentItem.modelData)
            }
            Keys.onDownPressed: allRow.forceActiveFocus()
            Keys.onEscapePressed: root.settingsRequested()
        }

        // All Applications row
        Text {
            text: "All Applications"
            font.pixelSize: Theme.fontTitle
            font.bold: true
            color: Theme.text
        }

        ListView {
            id: allRow
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.rowHeight
            orientation: ListView.Horizontal
            spacing: Theme.cardSpacing
            clip: true

            model: root.targets

            delegate: StreamCard {
                required property int index
                required property var modelData
                height: allRow.height - 20
                width: Theme.cardWidth
                target: modelData
                focus: index === allRow.currentIndex

                onActivated: root.streamRequested(modelData)

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        allRow.currentIndex = parent.index
                        parent.forceActiveFocus()
                    }
                    onDoubleClicked: root.streamRequested(parent.modelData)
                }
            }

            Keys.onReturnPressed: {
                if (allRow.currentItem)
                    root.streamRequested(allRow.currentItem.modelData)
            }
            Keys.onUpPressed: recentRow.forceActiveFocus()
            Keys.onEscapePressed: root.settingsRequested()
        }

        Item { Layout.fillHeight: true }

        Text {
            text: "A: Launch  |  B: Settings  |  ←→: Scroll"
            font.pixelSize: Theme.fontHint
            color: Theme.textDim
            Layout.alignment: Qt.AlignHCenter
            Layout.bottomMargin: 16
        }
    }
}
