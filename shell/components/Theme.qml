pragma Singleton
import QtQuick

Item {
    // === Theme Mode (persisted via SettingsStore) ===
    // "auto" (time-based), "light", "dark"
    readonly property string themeMode: SettingsStore.themeMode

    // === Input Mode (extracted to the InputMode singleton, #45 follow-up) ===
    // These are thin pass-throughs so call sites not yet migrated to InputMode
    // keep working. `mouseMode` re-exports the flag (readonly — write via
    // InputMode.enter/exitMouseMode); the helpers forward to InputMode. New code
    // should call InputMode.* directly. See InputMode.qml for the rationale.
    readonly property bool mouseMode: InputMode.mouseMode

    function enterMouseMode() {
        InputMode.enterMouseMode();
    }

    function exitMouseMode() {
        InputMode.exitMouseMode();
    }

    function pointerMoved(gx, gy) {
        InputMode.pointerMoved(gx, gy);
    }

    // === Controller Debug Overlay (persisted via SettingsStore) ===
    readonly property bool controllerDebug: SettingsStore.controllerDebug

    // === Accessibility settings (persisted via SettingsStore) ===
    readonly property bool reduceMotion: SettingsStore.reduceMotion  // #109
    readonly property real textScale: SettingsStore.textScale          // #110

    // === Home-screen widget toggles (persisted via SettingsStore) ===
    readonly property bool widgetSpotifyEnabled: SettingsStore.widgetSpotifyEnabled
    readonly property string widgetSpotifySize: SettingsStore.widgetSpotifySize
    readonly property bool widgetSpotifyHideFromRecent: SettingsStore.widgetSpotifyHideFromRecent
    readonly property bool widgetPlexEnabled: SettingsStore.widgetPlexEnabled
    readonly property string widgetPlexSize: SettingsStore.widgetPlexSize
    readonly property bool widgetPlexHideFromRecent: SettingsStore.widgetPlexHideFromRecent
    readonly property bool widgetRecentEnabled: SettingsStore.widgetRecentEnabled
    readonly property string widgetRecentSize: SettingsStore.widgetRecentSize
    readonly property bool widgetMoonlightEnabled: SettingsStore.widgetMoonlightEnabled
    // "small" (server cards) | "medium" (smaller Steam posters) | "large" (full Steam posters)
    readonly property string widgetMoonlightSize: SettingsStore.widgetMoonlightSize
    property int _currentHour: new Date().getHours()

    // === Auto theme schedule (#231, persisted via SettingsStore) ===
    // "auto" mode flips light↔dark on a configurable daily schedule.
    readonly property int autoDarkStart: SettingsStore.autoThemeDarkStart
    readonly property int autoLightStart: SettingsStore.autoThemeLightStart

    // True if `hour` falls in the dark window [darkStart, lightStart), handling
    // the usual midnight wrap (e.g. dark 20:00 → light 07:00).
    function _hourIsDark(hour, darkStart, lightStart) {
        if (darkStart === lightStart)
            return false; // degenerate schedule → never auto-dark
        if (darkStart < lightStart)
            return hour >= darkStart && hour < lightStart;
        return hour >= darkStart || hour < lightStart;
    }

    property bool darkMode: {
        if (themeMode === "dark")
            return true;
        if (themeMode === "light")
            return false;
        // auto: follow the configurable day/night schedule
        return _hourIsDark(_currentHour, autoDarkStart, autoLightStart);
    }

    // Re-evaluate auto mode every 60 seconds
    Timer {
        id: autoTimer
        interval: 60000
        running: themeMode === "auto"
        repeat: true
        onTriggered: {
            _currentHour = new Date().getHours();
        }
    }

    // === Base Palette (5 accent colors — shared across themes) ===
    readonly property color snow: "#f4f5f7"
    readonly property color crimson: "#c72138"
    readonly property color ember: "#e06236"
    readonly property color gold: "#d7a64b"
    readonly property color navy: "#304c7a"

    // === Structured Palette Objects ===
    // All theme-dependent colors grouped per mode. Adding a new theme =
    // one new object instead of editing 12 ternaries.
    readonly property var _darkPalette: ({
            background: "#111215",
            surface: "#33363f",
            surfaceHover: "#424650",
            surfaceBorder: "#4d525c",
            textPrimary: "#e6e4e0",
            textSecondary: "#c2bfba",
            textMuted: "#928e88" // WCAG: #928e88 on #111215 = 5.75:1 (AA normal >=4.5:1 PASS)
            ,
            cardBackground: "#2e3139",
            focusBorder: String(crimson),
            focusGlow: String(crimson) + "55",
            // Card focus (#237 follow-up): neutral, shadow-like cue instead of a
            // loud crimson ring — a thin cool-grey edge + soft cool-grey halo.
            cardFocusBorder: "#aab2c0",
            cardFocusGlow: "#5b6373" + "73",
            barBackground: "#111215",
            sidebarActive: "#424650"
        })
    readonly property var _lightPalette: ({
            background: String(snow),
            surface: "#ffffff",
            surfaceHover: "#ecedf0",
            surfaceBorder: "#dcdee3",
            textPrimary: "#1a2540",
            textSecondary: "#4a5568",
            textMuted: "#5a6473" // WCAG: #5a6473 on #f4f5f7 = 5.49:1 (AA normal >=4.5:1 PASS, #111)
            ,
            cardBackground: "#ffffff",
            focusBorder: String(crimson),
            focusGlow: String(crimson) + "55",
            // Card focus (#237 follow-up): a natural dark drop-shadow on light bg.
            cardFocusBorder: "#9aa0ac",
            cardFocusGlow: "#3a414f" + "59",
            barBackground: String(navy),
            sidebarActive: String(navy)
        })
    readonly property var palette: darkMode ? _darkPalette : _lightPalette

    // === Semantic Colors (aliases into active palette — no breaking changes) ===
    property color background: palette.background
    property color surface: palette.surface
    property color surfaceHover: palette.surfaceHover
    property color surfaceBorder: palette.surfaceBorder

    // Text hierarchy
    property color textPrimary: palette.textPrimary
    property color textSecondary: palette.textSecondary
    property color textMuted: palette.textMuted
    property color textOnDark: "#f4f5f7"
    property color textOnDarkMuted: "#d8d5d0"

    // Cards
    property color cardBackground: palette.cardBackground
    property color cardAccent: ember

    // Status
    readonly property color online: "#2d8a4e"
    property color offline: crimson
    property color warning: ember

    // Interactive
    property color focusBorder: palette.focusBorder
    property color focusGlow: palette.focusGlow
    property color cardFocusBorder: palette.cardFocusBorder
    property color cardFocusGlow: palette.cardFocusGlow
    property color barBackground: palette.barBackground
    property color sidebarActive: palette.sidebarActive
    property color sidebarText: "#e6e4e0"

    // === Layout — derived from Units.gridUnit (couch-readable, resolution-adaptive) ===
    readonly property int cardWidth: Math.round(Units.gridUnit * 11.11)
    readonly property int cardHeight: Math.round(Units.gridUnit * 8.89)
    readonly property int cardSpacing: Math.round(Units.gridUnit * 0.74)
    readonly property int cardRadius: Math.round(Units.gridUnit * 0.44)
    readonly property int padding: Math.round(Units.gridUnit * 0.89)
    readonly property int rowHeight: Math.round(Units.gridUnit * 10.0)
    readonly property int statusBarHeight: Math.round(Units.gridUnit * 2.22)

    // === Scrim ===
    // Canonical dim level for a modal overlay's full-screen backdrop. Modal
    // overlays previously hand-picked 0.7 / 0.85 ad hoc (drift); route them all
    // through DimmedBackdrop's default (this token) so the dim reads uniform.
    // Intentionally near-opaque surfaces (e.g. the power menu) set their own
    // higher dimLevel — this is the baseline, not a hard cap.
    readonly property real scrimOpacity: 0.85

    // === Font sizes — derived from Units.gridUnit (couch-readable, resolution-adaptive) ===
    // fontHero (hero clock) is intentionally unscaled — it owns the layout.
    // All text-content tiers scale by textScale so users can enlarge body text (#110).
    readonly property int fontHero: Math.round(Units.gridUnit * 2.22)
    readonly property int fontTitle: Math.round(Units.gridUnit * 1.04 * textScale)
    readonly property int fontBody: Math.round(Units.gridUnit * 0.74 * textScale)
    readonly property int fontSmall: Math.round(Units.gridUnit * 0.59 * textScale)
    readonly property int fontStatus: Math.round(Units.gridUnit * 0.74 * textScale)
    readonly property int fontHint: Math.round(Units.gridUnit * 0.67 * textScale)
    readonly property int fontCaption: Math.round(Units.gridUnit * 0.52 * textScale)
    readonly property int fontXSmall: Math.round(Units.gridUnit * 0.44 * textScale)
}
