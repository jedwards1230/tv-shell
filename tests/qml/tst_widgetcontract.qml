import QtQuick
import QtTest
import Quickshell.Services.Mpris
import widgets.apps
import widgets.plex
import widgets.moonlight
import widgets.nowplaying
import widgets.steam

// Widget-contract conformance test — loads the FIVE REAL home widgets
// (AppsWidget / PlexWidget / MoonlightWidget / NowPlayingWidget / SteamWidget)
// through the headless harness (run.sh mirrors the real widget subtree into
// .build so their own relative imports resolve; Quickshell-backed clients +
// pure-visual leaf CARDS are stubbed — see tests/qml/README.md + widgetstubs/).
//
// The assertions are BEHAVIORAL and refactor-agnostic: they never check that a
// named function/signal exists on the base, only that each widget honours the
// duck-typed home-tile contract HomeScreen + NavigableRow + WidgetHost query:
//
//   1. It instantiates with no QML load errors.
//   2. focusFirstChild() returns a boolean without throwing.
//   3. canFocus is a boolean.
//   4. widgetEnabled = false collapses wantVisible to false (made wantVisible
//      true first where feasible; a widget whose visibility is gated on live
//      remote health that can't be faked headlessly asserts the collapse only).
//   5. No focus trap — a synthetic Up/Down moves focus OFF the widget onto the
//      wired previousRow/nextRow neighbours. A widget that can't be focused
//      headlessly instead proves it is SKIPPED by the chain (canFocus false), so
//      it structurally cannot trap focus.
//   6. IF the widget exposes an ensureVisibleRequested signal (feature-detected,
//      NOT hardcoded — the base may or may not carry it on a given branch),
//      focusing it fires that signal.
TestCase {
    id: testCase
    name: "WidgetContract"
    when: windowShown
    visible: true
    width: 900
    height: 700

    // A rig: two focus-sentinel neighbours (canFocus true, record activeFocus)
    // with the widget between them in a Loader. The widget's previousRow/nextRow
    // are wired to the sentinels on load, exactly as WidgetHost wires the real
    // vertical focus chain.
    Component {
        id: rigComp
        Item {
            id: rigRoot
            width: 900
            height: 700

            property Component widgetComp: null
            property Item widget: null
            property alias prevN: pn
            property alias nextN: nn
            property alias loader: wl

            FocusScope {
                id: pn
                width: 20
                height: 20
                property bool canFocus: true
                property Item previousRow: null
                property Item nextRow: null
                function focusFirstChild() {
                    forceActiveFocus();
                    return true;
                }
            }
            Loader {
                id: wl
                y: 40
                width: 860
                sourceComponent: rigRoot.widgetComp
                onLoaded: {
                    rigRoot.widget = item;
                    item.previousRow = pn;
                    item.nextRow = nn;
                }
            }
            FocusScope {
                id: nn
                y: 660
                width: 20
                height: 20
                property bool canFocus: true
                property Item previousRow: null
                property Item nextRow: null
                function focusFirstChild() {
                    forceActiveFocus();
                    return true;
                }
            }
        }
    }

    // A fake MPRIS player so NowPlaying can be made visible + focusable headlessly.
    // Carries only the surface MprisPlayerBase reads; the transport methods are
    // never invoked (the test presses Up/Down, not Return).
    Component {
        id: fakePlayerComp
        QtObject {
            property bool isPlaying: true
            property bool canGoPrevious: true
            property bool canGoNext: true
            property bool canTogglePlaying: true
            property bool canPlay: true
            property bool canPause: true
            property string desktopEntry: "fake"
            property string identity: "Fake Player"
            function previous() {
            }
            function next() {
            }
            function togglePlaying() {
            }
        }
    }

    Component {
        id: spyComp
        SignalSpy {}
    }

    Component {
        id: appsComp
        AppsWidget {}
    }
    Component {
        id: plexComp
        PlexWidget {}
    }
    Component {
        id: moonlightComp
        MoonlightWidget {}
    }
    Component {
        id: nowPlayingComp
        NowPlayingWidget {}
    }
    Component {
        id: steamComp
        SteamWidget {}
    }

    // Reset the shared Mpris stub so test order can't leak an injected player.
    function cleanup() {
        Mpris.players = null;
    }

    // Focus the widget's first internal row (or the widget itself when it is a
    // single focus stop with no firstRow), so an Up press exercises the top-edge
    // hand-off to previousRow.
    function _focusFirstRow(w) {
        var r = w.firstRow;
        if (r && typeof r.focusFirstChild === "function") {
            r.focusFirstChild();
            return;
        }
        if (r) {
            r.forceActiveFocus();
            return;
        }
        w.focusFirstChild();
    }

    // Shared contract runner. opts = { name, reachable, arrange? }.
    //   reachable — can the widget be made visible + focusable headlessly?
    //   arrange(w) — inject the minimal content/singleton state to do so.
    function _run(widgetComp, opts) {
        var name = opts.name;
        var rig = createTemporaryObject(rigComp, testCase, {
            "widgetComp": widgetComp
        });
        verify(rig, name + ": rig created");

        // 1. Instantiates with no QML load errors.
        compare(rig.loader.status, Loader.Ready, name + ": Loader.Ready (widget loaded, no QML errors)");
        var w = rig.widget;
        verify(w !== null, name + ": widget instance is non-null");

        if (opts.arrange)
            opts.arrange(w);

        // 2. focusFirstChild() returns a boolean without throwing.
        var ff = w.focusFirstChild();
        compare(typeof ff, "boolean", name + ": focusFirstChild() returns a boolean");

        // 3. canFocus is a boolean.
        compare(typeof w.canFocus, "boolean", name + ": canFocus is a boolean");

        // 4. widgetEnabled = false collapses wantVisible.
        if (opts.reachable) {
            verify(w.wantVisible, name + ": wantVisible is true once content is present");
        } else {
            console.log("SKIP-NOTE [" + name + "]: wantVisible cannot be driven true headlessly (gated on live service health); asserting the enabled->disabled collapse only.");
        }
        w.widgetEnabled = false;
        compare(w.wantVisible, false, name + ": widgetEnabled=false collapses wantVisible to false");
        w.widgetEnabled = true;

        // 5. No focus trap.
        if (opts.reachable) {
            // Down off the naturally-focused row hands off to nextRow.
            rig.prevN.forceActiveFocus();
            verify(w.focusFirstChild(), name + ": focusFirstChild() succeeds when reachable");
            verify(w.regionFocused, name + ": widget holds focus after focusFirstChild()");
            keyClick(Qt.Key_Down);
            verify(rig.nextN.activeFocus, name + ": Down leaves the widget onto nextRow (no focus trap)");

            // Up off the first row hands off to previousRow.
            rig.nextN.forceActiveFocus();
            _focusFirstRow(w);
            verify(w.regionFocused, name + ": widget holds focus on its first row");
            keyClick(Qt.Key_Up);
            verify(rig.prevN.activeFocus, name + ": Up leaves the widget onto previousRow (no focus trap)");
        } else {
            // Not focusable headlessly → the vertical-chain walk skips it
            // (canFocus false), so it structurally cannot trap focus.
            compare(w.canFocus, false, name + ": non-focusable headless → chain skips it (cannot trap focus)");
            compare(w.focusFirstChild(), false, name + ": focusFirstChild() returns false with no focusable content");
        }

        // 6. ensureVisibleRequested fires on focus (feature-detected).
        // QML exposes a signal as a callable function object, so `typeof` a
        // present signal is "function" (verified in this runner) and "undefined"
        // when the widget's base doesn't declare it — this is a real detection,
        // NOT dead code: it's true for Apps/Moonlight and false for NowPlaying
        // (MprisPlayerBase has no such signal on this branch).
        var hasEnsure = (typeof w.ensureVisibleRequested === "function");
        if (hasEnsure && opts.reachable) {
            var spy = createTemporaryObject(spyComp, testCase, {
                "target": w,
                "signalName": "ensureVisibleRequested"
            });
            verify(spy.valid, name + ": ensureVisibleRequested SignalSpy is valid");
            rig.prevN.forceActiveFocus();
            spy.clear();
            w.focusFirstChild();
            tryVerify(function () {
                return spy.count > 0;
            }, 1000, name + ": focusing the widget fires ensureVisibleRequested");
        } else if (hasEnsure) {
            console.log("SKIP-NOTE [" + name + "]: exposes ensureVisibleRequested but can't be focused headlessly; firing not asserted.");
        } else {
            console.log("NOTE [" + name + "]: no ensureVisibleRequested signal (refactor-agnostic — assertion skipped).");
        }
    }

    // === Per-widget rows =====================================================

    // Apps: injecting a recent-model row makes it visible + focusable (rail).
    function test_apps() {
        _run(appsComp, {
            "name": "AppsWidget",
            "reachable": true,
            "arrange": function (w) {
                w.model = [
                    {
                        "name": "App One",
                        "exec": "app1",
                        "icon": "",
                        "comment": "",
                        "wmClass": "",
                        "running": false
                    }
                ];
            }
        });
    }

    // Moonlight: size "small" (server view) + one target makes it visible +
    // focusable without needing live Steam-library health.
    function test_moonlight() {
        _run(moonlightComp, {
            "name": "MoonlightWidget",
            "reachable": true,
            "arrange": function (w) {
                w.size = "small";
                w.targets = [
                    {
                        "host": "10.0.0.1",
                        "name": "Desktop",
                        "app": "Steam Big Picture"
                    }
                ];
            }
        });
    }

    // Now Playing: inject a fake MPRIS player so hasPlayer → the widget shows and
    // takes focus as a single stop.
    function test_nowplaying() {
        _run(nowPlayingComp, {
            "name": "NowPlayingWidget",
            "reachable": true,
            "arrange": function (w) {
                Mpris.players = {
                    "values": [fakePlayerComp.createObject(testCase)]
                };
            }
        });
    }

    // Steam: the local-Steam poster widget. Visibility is gated on the hosted
    // SteamLibraryView's live `steam-library` service health (same as Moonlight's
    // library view / Plex), which needs the live daemon health bus — not fakeable
    // headlessly. Covered for load + contract shape; the focus hand-off is proven
    // structurally (the chain skips a non-focusable widget) rather than by a
    // synthetic key event. This is the widget whose cross-PR drift (a leaf
    // re-declaring the base's ensureVisibleRequested signal) bricked the shell;
    // covering it here makes that exact load failure CI-catchable.
    function test_steam() {
        _run(steamComp, {
            "name": "SteamWidget",
            "reachable": false
        });
    }

    // Plex: visibility is gated on the ServiceMonitor reporting ok/degraded, which
    // needs the live daemon health bus — not fakeable headlessly. Covered for
    // load + contract shape; the focus hand-off is proven structurally (the chain
    // skips a non-focusable widget) rather than by a synthetic key event.
    function test_plex() {
        _run(plexComp, {
            "name": "PlexWidget",
            "reachable": false
        });
    }
}
