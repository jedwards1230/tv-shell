import QtQuick
import "../"

// Self-ticking clock/date label. Replaces the Text+Timer pair that HomeScreen's
// hero and NavigationDrawer's header each hand-rolled (identical "h:mm AP" clock
// + "dddd, MMMM d" date, ticking on 1s/60s timers).
//
//   ClockText { kind: "time" }                      // 1s "h:mm AP" clock
//   ClockText { kind: "date" }                      // 60s "dddd, MMMM d" date
//   ClockText { kind: "time"; running: root.opened } // gate the ticker
//
// Style (font/color) is set by the caller exactly as on a plain Text. The ticker
// only runs while `running` is true (default true); the drawer gates it on its
// open state so a hidden drawer isn't waking on a timer.
Text {
    id: clock

    // "time" → toLocaleTimeString on a 1s tick; "date" → toLocaleDateString on a
    // 60s tick. Both default to the formats the two call sites used.
    property string kind: "time"
    property string format: kind === "date" ? "dddd, MMMM d" : "h:mm AP"

    // Tick cadence. Defaults follow `kind`; override for an unusual format.
    property int interval: kind === "date" ? 60000 : 1000

    // Drive the ticker. Set false (or bind to a visibility flag) to park it.
    property bool running: true

    function _refresh() {
        let now = new Date();
        clock.text = kind === "date" ? now.toLocaleDateString(Qt.locale(), format) : now.toLocaleTimeString(Qt.locale(), format);
    }

    Timer {
        interval: clock.interval
        running: clock.running
        repeat: true
        triggeredOnStart: true
        onTriggered: clock._refresh()
    }
}
