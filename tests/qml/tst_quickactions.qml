import QtQuick
import QtTest
import components

// Headless layout + D-pad-navigation tests for QuickActions (the top-right
// status strip / nav-drawer action row). Runs under qmltestrunner with
// QT_QPA_PLATFORM=offscreen — no Quickshell, no GPU. The real QuickActions /
// QuickActionButton / CountBadge are exercised against stub singletons (see
// tests/qml/stubs + tests/qml/README.md).
// These lock the two invariants the controller-first design depends on:
//   1. every action is reachable by D-pad (Left/Right) and clamps at the ends,
//   2. Return on each index fires the right request signal (incl. the
//      auto -> light -> dark -> auto theme-toggle cycle),
// plus Up/Down/Escape behaviour and the strip's implicit sizing.
TestCase {
    id: testCase
    name: "QuickActions"
    when: windowShown
    width: 900
    height: 240
    visible: true

    Component {
        id: qaComp
        QuickActions {}
    }

    // Fresh instance per test; reset shared singleton state first.
    function newQA() {
        SettingsStore.setThemeMode("auto");
        InputMode.exitMouseMode();
        var qa = createTemporaryObject(qaComp, testCase);
        verify(qa, "QuickActions instantiated");
        qa.forceActiveFocus();
        verify(qa.activeFocus, "QuickActions has active focus");
        return qa;
    }

    // --- Vocabulary / sizing -----------------------------------------------
    function test_index_vocabulary() {
        var qa = newQA();
        // Index map is contract for the home screen + docs/qa-screenshot-views.md
        // ("0=Notifications, 1=Settings, 2=Theme toggle, 3=Network, 4=Volume,
        // 5=Power"). Lock the count + labels.
        compare(qa._iconCount, 6);
        compare(qa._labels, ["Notifications", "Settings", "Theme", "Network", "Volume", "Power"]);
    }

    function test_initial_index_is_zero() {
        var qa = newQA();
        compare(qa.currentIndex, 0);
    }

    function test_implicit_height_tracks_icon_and_label() {
        var qa = newQA();
        compare(qa.implicitHeight, qa.iconSize + qa._labelHeight);
        verify(qa.implicitHeight > 0);
        verify(qa.implicitWidth > 0);
    }

    // --- D-pad navigation + clamping ---------------------------------------
    function test_right_advances_and_clamps_at_end() {
        var qa = newQA();
        for (var i = 1; i <= 5; ++i) {
            keyClick(Qt.Key_Right);
            compare(qa.currentIndex, i);
        }
        // Already at the last action — Right must not wrap or overrun.
        keyClick(Qt.Key_Right);
        compare(qa.currentIndex, 5);
    }

    function test_left_decrements_and_clamps_at_start() {
        var qa = newQA();
        qa.currentIndex = 5;
        for (var i = 4; i >= 0; --i) {
            keyClick(Qt.Key_Left);
            compare(qa.currentIndex, i);
        }
        // Already at the first action — Left must not wrap below 0.
        keyClick(Qt.Key_Left);
        compare(qa.currentIndex, 0);
    }

    // --- Activation signals (A / Return) -----------------------------------
    function test_return_on_notifications() {
        var qa = newQA();
        var spy = createTemporaryObject(spyComp, testCase, {
            "target": qa,
            "signalName": "notificationCenterRequested"
        });
        qa.currentIndex = 0;
        keyClick(Qt.Key_Return);
        compare(spy.count, 1);
    }

    function test_return_on_settings() {
        var qa = newQA();
        var spy = createTemporaryObject(spyComp, testCase, {
            "target": qa,
            "signalName": "settingsRequested"
        });
        qa.currentIndex = 1;
        keyClick(Qt.Key_Return);
        compare(spy.count, 1);
    }

    function test_return_on_network() {
        var qa = newQA();
        var spy = createTemporaryObject(spyComp, testCase, {
            "target": qa,
            "signalName": "networkRequested"
        });
        qa.currentIndex = 3;
        keyClick(Qt.Key_Return);
        compare(spy.count, 1);
        // The overlay anchors itself beside the glyph, so the rect must carry
        // the glyph's real dimensions (w/h come from item.width/height = iconSize).
        var rect = spy.signalArguments[0][0];
        verify(rect !== null && rect !== undefined);
        compare(rect.w, qa.iconSize);
        compare(rect.h, qa.iconSize);
    }

    function test_return_on_volume() {
        var qa = newQA();
        var spy = createTemporaryObject(spyComp, testCase, {
            "target": qa,
            "signalName": "volumeRequested"
        });
        qa.currentIndex = 4;
        keyClick(Qt.Key_Return);
        compare(spy.count, 1);
        var rect = spy.signalArguments[0][0];
        verify(rect !== null && rect !== undefined);
        compare(rect.w, qa.iconSize);
        compare(rect.h, qa.iconSize);
    }

    function test_return_on_power() {
        var qa = newQA();
        var spy = createTemporaryObject(spyComp, testCase, {
            "target": qa,
            "signalName": "powerRequested"
        });
        qa.currentIndex = 5;
        keyClick(Qt.Key_Return);
        compare(spy.count, 1);
    }

    // --- Theme toggle cycle (index 2) --------------------------------------
    function test_theme_toggle_cycles_auto_light_dark() {
        var qa = newQA();
        qa.currentIndex = 2;
        compare(SettingsStore.themeMode, "auto");
        keyClick(Qt.Key_Return);
        compare(SettingsStore.themeMode, "light");
        keyClick(Qt.Key_Return);
        compare(SettingsStore.themeMode, "dark");
        keyClick(Qt.Key_Return);
        compare(SettingsStore.themeMode, "auto");
    }

    // --- Up / Down / Escape ------------------------------------------------
    function test_down_requests_focus_handoff() {
        var qa = newQA();
        var spy = createTemporaryObject(spyComp, testCase, {
            "target": qa,
            "signalName": "focusDownRequested"
        });
        keyClick(Qt.Key_Down);
        compare(spy.count, 1);
    }

    function test_up_requests_focus_handoff() {
        var qa = newQA();
        var spy = createTemporaryObject(spyComp, testCase, {
            "target": qa,
            "signalName": "focusUpRequested"
        });
        keyClick(Qt.Key_Up);
        compare(spy.count, 1);
    }

    function test_escape_opens_settings_by_default() {
        var qa = newQA();
        verify(qa.escapeRequestsSettings);
        var spy = createTemporaryObject(spyComp, testCase, {
            "target": qa,
            "signalName": "settingsRequested"
        });
        keyClick(Qt.Key_Escape);
        compare(spy.count, 1);
    }

    Component {
        id: spyComp
        SignalSpy {}
    }
}
