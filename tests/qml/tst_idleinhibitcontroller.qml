import QtQuick
import QtTest
import Quickshell.Services.Mpris
import components

// Headless truth-table test for IdleInhibitController (#195) — the pure policy
// deciding WHETHER the shell asserts a Wayland idle-inhibitor. Runs under
// qmltestrunner with QT_QPA_PLATFORM=offscreen against the stub Mpris singleton
// (widgetstubs/qml/Quickshell/Services/Mpris) — no Quickshell runtime.
//
// Locks the two invariants #195 depends on:
//   - shouldInhibit is TRUE in streaming (any media state) and in appRunning ONLY
//     while an MPRIS player is Playing.
//   - shouldInhibit is FALSE on every static/idle state (idle/launching/
//     reconnecting) even with a player Playing, and in appRunning with no player
//     or only a paused player.
// mediaPlaying is asserted alongside so the null-safe player scan is covered too.
TestCase {
    id: testCase
    name: "IdleInhibitController"
    when: windowShown
    visible: true
    width: 200
    height: 200

    Component {
        id: ctrlComp
        IdleInhibitController {}
    }

    // Reset the shared Mpris stub between tests so an injected player can't leak.
    function cleanup() {
        Mpris.players = null;
    }

    // Inject a players list of plain JS objects — the stub's default `players` is
    // null, matching production; the controller reads Mpris.players.values and each
    // entry's isPlaying (see IdleInhibitController.mediaPlaying).
    function _setPlayers(playing) {
        var vals = [];
        for (var i = 0; i < playing.length; i++)
            vals.push({
                "isPlaying": playing[i]
            });
        Mpris.players = {
            "values": vals
        };
    }

    function _make(state) {
        var c = createTemporaryObject(ctrlComp, testCase, {
            "shellState": state
        });
        verify(c, "controller instantiated for state=" + state);
        return c;
    }

    // --- mediaPlaying reflects injected players -----------------------------
    function test_mediaplaying_null_players() {
        var c = _make("appRunning");
        compare(c.mediaPlaying, false, "no players on the bus -> mediaPlaying false");
    }

    function test_mediaplaying_paused_only() {
        _setPlayers([false]);
        var c = _make("appRunning");
        compare(c.mediaPlaying, false, "only a paused player -> mediaPlaying false");
    }

    function test_mediaplaying_one_playing() {
        _setPlayers([false, true]);
        var c = _make("appRunning");
        compare(c.mediaPlaying, true, "any playing player -> mediaPlaying true");
    }

    // --- streaming: inhibit regardless of media -----------------------------
    function test_streaming_inhibits_without_media() {
        var c = _make("streaming");
        compare(c.mediaPlaying, false, "no players");
        compare(c.shouldInhibit, true, "streaming inhibits even with no media playing");
    }

    function test_streaming_inhibits_with_media() {
        _setPlayers([true]);
        var c = _make("streaming");
        compare(c.shouldInhibit, true, "streaming inhibits with media playing");
    }

    // --- appRunning: inhibit ONLY while a player is Playing ------------------
    function test_apprunning_playing_inhibits() {
        _setPlayers([true]);
        var c = _make("appRunning");
        compare(c.shouldInhibit, true, "appRunning + playing player inhibits");
    }

    function test_apprunning_no_player_does_not_inhibit() {
        var c = _make("appRunning");
        compare(c.shouldInhibit, false, "appRunning + no player does not inhibit");
    }

    function test_apprunning_paused_does_not_inhibit() {
        _setPlayers([false]);
        var c = _make("appRunning");
        compare(c.shouldInhibit, false, "appRunning + paused player does not inhibit");
    }

    // --- static/idle states never inhibit, even with media playing ----------
    function test_idle_never_inhibits() {
        _setPlayers([true]);
        var c = _make("idle");
        compare(c.shouldInhibit, false, "idle never inhibits, even with media playing");
    }

    function test_launching_never_inhibits() {
        _setPlayers([true]);
        var c = _make("launching");
        compare(c.shouldInhibit, false, "launching never inhibits, even with media playing");
    }

    function test_reconnecting_never_inhibits() {
        _setPlayers([true]);
        var c = _make("reconnecting");
        compare(c.shouldInhibit, false, "reconnecting never inhibits, even with media playing");
    }
}
