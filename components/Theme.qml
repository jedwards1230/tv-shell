pragma Singleton
import QtQuick

QtObject {
    // === Base Palette (5 colors) ===
    // Save reference: /opt/game-shell/config/palette.md
    readonly property color snow: "#f4f5f7"        // Light background
    readonly property color crimson: "#c72138"      // Primary accent (red)
    readonly property color ember: "#e06236"        // Secondary accent (orange)
    readonly property color gold: "#d7a64b"         // Tertiary accent (gold)
    readonly property color navy: "#304c7a"         // Dark accent (navy blue)

    // === Semantic Colors (derived from base palette) ===
    readonly property color background: snow
    readonly property color surface: "#ffffff"
    readonly property color surfaceHover: "#ecedf0"
    readonly property color surfaceBorder: "#dcdee3"

    // Text hierarchy — all derived from navy
    readonly property color textPrimary: "#1a2540"  // Near-black, navy-tinted
    readonly property color textSecondary: "#4a5568" // Medium gray, navy-tinted
    readonly property color textMuted: "#8892a4"    // Light gray, navy-tinted
    readonly property color textOnDark: "#f4f5f7"   // Snow on dark backgrounds
    readonly property color textOnDarkMuted: "#c8ccd4" // Muted snow

    // Status
    readonly property color online: "#2d8a4e"       // Green (independent of palette)
    readonly property color offline: crimson
    readonly property color warning: ember

    // Interactive
    readonly property color focusBorder: crimson
    readonly property color focusGlow: "#c7213833"  // Crimson at 20% opacity
    readonly property color sidebarActive: navy
    readonly property color sidebarText: "#f4f5f7"
    readonly property color barBackground: navy

    // Card accents — use ember for decorative elements instead of gold on text
    readonly property color cardAccent: ember

    // === Layout — couch-readable at 4K (10-foot UI) ===
    readonly property int cardWidth: 640
    readonly property int cardHeight: 360
    readonly property int cardSpacing: 48
    readonly property int cardRadius: 24
    readonly property int statusBarHeight: 120
    readonly property int padding: 48
    readonly property int rowHeight: 420

    // === Font sizes — couch-readable at 4K ===
    readonly property int fontHero: 72
    readonly property int fontTitle: 56
    readonly property int fontBody: 40
    readonly property int fontSmall: 32
    readonly property int fontStatus: 40
    readonly property int fontHint: 36
}
