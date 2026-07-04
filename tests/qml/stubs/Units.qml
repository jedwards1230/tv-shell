pragma Singleton
import QtQuick

// Test stub for the production `Units` singleton (shell/components/Units.qml),
// which reads Quickshell.screens for its grid unit. Fixed values here keep
// layout assertions deterministic under offscreen. See tests/qml/README.md.
Item {
    property int spacingXS: 3
    property int spacingSM: 6
    property int spacingMD: 12
    property int spacingLG: 18
    property int spacingXL: 24

    property int borderThin: 1
    property int borderMedium: 2

    // Consumed by WakeCard/SteamCard (FocusFrame radius + poster/glyph sizing).
    // Fixed plausible values keep offscreen layout deterministic (real ones are
    // gridUnit-derived: radiusMD ≈ gridUnit*0.30, iconSizeLG ≈ gridUnit*2.22).
    property int radiusMD: 11
    property int iconSizeLG: 80
}
