pragma Singleton
import Quickshell
import QtQuick

Item {
    id: units

    // Single-screen kiosk — uses the primary screen's pixel height.
    readonly property int screenHeight: Quickshell.screens.length > 0 ? Quickshell.screens[0].height : 2160

    readonly property int gridUnit: Math.round(screenHeight / 40)

    readonly property int spacingXS: Math.round(gridUnit * 0.15)
    readonly property int spacingSM: Math.round(gridUnit * 0.30)
    readonly property int spacingMD: Math.round(gridUnit * 0.44)
    readonly property int spacingLG: Math.round(gridUnit * 0.59)
    readonly property int spacingXL: Math.round(gridUnit * 0.89)

    readonly property int radiusSM: Math.round(gridUnit * 0.15)
    readonly property int radiusMD: Math.round(gridUnit * 0.30)
    readonly property int radiusLG: Math.round(gridUnit * 0.44)
    readonly property int radiusXL: Math.round(gridUnit * 0.59)

    readonly property int borderThin: Math.max(1, Math.round(gridUnit * 0.037))
    readonly property int borderMedium: Math.max(2, Math.round(gridUnit * 0.056))
    readonly property int borderThick: Math.max(3, Math.round(gridUnit * 0.11))

    readonly property int iconSizeSM: Math.round(gridUnit * 0.59)
    readonly property int iconSizeMD: Math.round(gridUnit * 1.19)
    readonly property int iconSizeLG: Math.round(gridUnit * 2.22)
    readonly property int iconSizeXL: Math.round(gridUnit * 4.44)

    // Settings-panel chrome — previously hardcoded to the 4K reference (560/100/
    // 80 px). Expressed off gridUnit so the panel scales on non-4K screens; the
    // multipliers are chosen to land on the original pixel values at 2160p.
    readonly property int sidebarWidth: Math.round(gridUnit * 10.37)
    readonly property int settingsRowHeight: Math.round(gridUnit * 1.85)
    readonly property int settingsHintHeight: Math.round(gridUnit * 1.48)
}
