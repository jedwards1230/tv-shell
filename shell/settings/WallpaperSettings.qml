import QtQuick
import QtQuick.Layouts
import Qt.labs.folderlistmodel
import "../components"
import "../components/lib"

// Wallpaper picker (#29). Browses ~/.config/tv-shell/wallpapers/ read-only via
// FolderListModel and lets the user choose an image (or "None") as the
// home-screen background. Persists SettingsStore.wallpaperPath as a plain
// filesystem path (not a file:// URL) — keeps settings.json portable/readable;
// HomeScreen re-derives the file:// URL for the Image source itself.
SettingsPageBase {
    id: root
    hintText: "A: Select   B: Back"

    function focusFirst() {
        grid.forceActiveFocus();
    }

    FolderListModel {
        id: folderModel
        folder: "file://" + Paths.tvShellConfigDir + "/wallpapers"
        nameFilters: ["*.jpg", "*.jpeg", "*.png", "*.webp", "*.bmp"]
        caseSensitive: false
        showDirs: false
        sortField: FolderListModel.Name
    }

    // Flat picker model: a synthetic "None" tile followed by one entry per
    // discovered image. Rebuilt whenever the folder listing settles or its
    // count changes (FolderListModel's model roles aren't a plain JS array, and
    // NavigableGrid needs one flat indexable model for its row/column math).
    property var items: [
        {
            none: true
        }
    ]

    function _rebuildItems() {
        var list = [
            {
                none: true
            }
        ];
        for (var i = 0; i < folderModel.count; i++) {
            var fileUrl = folderModel.get(i, "fileURL").toString();
            // Strip the file:// scheme — settings.json stores a plain
            // filesystem path (portable/readable), not a URL.
            var plainPath = fileUrl.replace(/^file:\/\//, "");
            list.push({
                none: false,
                name: folderModel.get(i, "fileName"),
                path: plainPath,
                url: fileUrl
            });
        }
        root.items = list;
    }

    // Debounced via Qt.callLater — status + count both fire as the folder
    // listing settles; Qt.callLater dedupes the multiple fires into one rebuild
    // per event-loop turn.
    Connections {
        target: folderModel
        function onStatusChanged() {
            Qt.callLater(root._rebuildItems);
        }
        function onCountChanged() {
            Qt.callLater(root._rebuildItems);
        }
    }

    Component.onCompleted: root._rebuildItems()

    // Select the tile at `idx` — the None tile clears the wallpaper, an image
    // tile sets it. Shared by both grid.activated (A/Return) and a delegate
    // click.
    function _selectAt(idx) {
        var sel = root.items[idx];
        if (!sel)
            return;
        if (sel.none)
            SettingsStore.setWallpaperPath("");
        else
            SettingsStore.setWallpaperPath(sel.path);
    }

    SectionHeader {
        text: "Wallpaper"
    }

    Text {
        visible: root.items.length <= 1
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        text: "Drop images into ~/.config/tv-shell/wallpapers/ to choose a wallpaper."
        font.pixelSize: Theme.fontHint
        color: Theme.textMuted
    }

    NavigableGrid {
        id: grid
        Layout.fillWidth: true
        cellWidth: Theme.cardWidth
        cellHeight: Theme.cardHeight
        spacing: Theme.cardSpacing
        model: root.items

        onActivated: root._selectAt(grid.currentIndex)
        // NavigableGrid accepts B/Escape and only emits `escaped` (no back-nav
        // itself), so re-emit backRequested → SettingsApp routes it through the
        // unified _back() (grid → sidebar → Home). Without this the grid — the
        // page's only focusable control — would strand focus here.
        onEscaped: root.backRequested()

        delegate: Item {
            id: tile
            required property int index
            required property var modelData
            width: grid.cellWidth
            height: grid.cellHeight
            focus: index === grid.currentIndex

            readonly property bool isSelected: modelData.none ? SettingsStore.wallpaperPath === "" : modelData.path === SettingsStore.wallpaperPath

            FocusFrame {
                id: frame
                anchors.fill: parent
                radius: Theme.cardRadius
                focused: tile.focus && grid.activeFocus
                restBorderColor: tile.isSelected ? Theme.focusBorder : Theme.surfaceBorder
                restBorderWidth: tile.isSelected ? 3 : Units.borderThin

                Image {
                    anchors.fill: parent
                    visible: !modelData.none
                    source: modelData.none ? "" : modelData.url
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: true
                }

                Text {
                    anchors.centerIn: parent
                    visible: modelData.none
                    horizontalAlignment: Text.AlignHCenter
                    text: "None\n(solid color)"
                    font.pixelSize: Theme.fontSmall
                    font.bold: true
                    color: Theme.textPrimary
                }

                // Persistent "currently selected" marker, distinct from the
                // (transient) focus ring above.
                Text {
                    visible: tile.isSelected
                    anchors.top: parent.top
                    anchors.right: parent.right
                    anchors.margins: Units.spacingSM
                    text: "✓"
                    font.pixelSize: Theme.fontBody
                    font.bold: true
                    color: Theme.focusBorder
                }
            }

            MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    InputMode.enterMouseMode();
                    grid.currentIndex = tile.index;
                    grid.forceActiveFocus();
                    root._selectAt(tile.index);
                }
            }
        }
    }
}
