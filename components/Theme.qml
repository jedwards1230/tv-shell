pragma Singleton
import QtQuick

QtObject {
    // Palette: light background, navy/red/orange/gold accents
    readonly property color background: "#f4f5f7"
    readonly property color surface: "#ffffff"
    readonly property color surfaceHover: "#e8eaef"
    readonly property color primary: "#304c7a"
    readonly property color accent: "#c72138"
    readonly property color accentOrange: "#e06236"
    readonly property color accentGold: "#d7a64b"
    readonly property color text: "#1a1a2e"
    readonly property color textDim: "#5a6070"
    readonly property color online: "#2d8a4e"
    readonly property color offline: "#c72138"
    readonly property color warning: "#d7a64b"

    // Layout — couch-readable at 4K (10-foot UI)
    readonly property int cardWidth: 640
    readonly property int cardHeight: 360
    readonly property int cardSpacing: 48
    readonly property int cardRadius: 24
    readonly property int statusBarHeight: 120
    readonly property int padding: 48
    readonly property int rowHeight: 420

    // Font sizes — couch-readable at 4K
    readonly property int fontHero: 72
    readonly property int fontTitle: 56
    readonly property int fontBody: 40
    readonly property int fontSmall: 32
    readonly property int fontStatus: 40
    readonly property int fontHint: 36
}
