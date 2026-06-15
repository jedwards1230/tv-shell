import QtQuick
import "../"

// Non-visual remote-service health/data source. The reusable QML half of the
// daemon's service-health bus (daemon/src/service_health.rs): it turns the
// daemon's health vocabulary (`disabled`/`ok`/`unreachable`/`error`) into bound
// properties any widget can render, so widgets stop conflating "server down"
// with "no data" and can show a graceful notice instead of collapsing silently.
//
// Two source modes (a given instance uses one):
//
//   • Broadcast mode (set `healthKey`) — subscribe to the daemon's
//     `health:<json>` events and adopt the `status` for the matching service.
//     Authoritative and live for any number of widgets (the daemon polls once).
//     Pair with `dataCommand` to also fetch the service's payload (e.g.
//     `plex-hubs`) — primed once on start, refreshed on a timer while healthy,
//     and re-fetched immediately when the service recovers to `ok`.
//
//   • Poll mode (leave `healthKey` empty, set `dataCommand`) — poll a
//     request/response command on a timer; `status` is read from the reply's
//     own `status` field (e.g. `sunshine-status`, per stream card). For
//     per-instance services the daemon doesn't globally poll.
//
// Either way the consumer binds to `status` / `ok` / `degraded` / `disabled`
// and `data`, and pairs with `ServiceStatusNotice` for the degraded UX.
Item {
    id: mon

    // --- Configuration ---

    // Broadcast mode: the service name to match in `health:<json>` events
    // (e.g. "plex"). Empty → poll mode.
    property string healthKey: ""

    // Request/response command whose reply populates `data` (and, in poll mode,
    // `status` from the reply's `status` field). Empty → status-only monitor.
    property string dataCommand: ""

    // Data refresh cadence (ms). Drives the poll-mode status poll and the
    // broadcast-mode data refresh.
    property int dataIntervalMs: 60000

    // Host gate for the data poll — set false to pause polling (e.g. while the
    // widget is hidden, or the card's host is offline / a stream is live). The
    // broadcast subscription stays connected regardless; only the timer pauses.
    property bool active: true

    // --- Observed state ---

    // "unknown" until the first health event / reply arrives, then one of the
    // daemon's tokens: "ok" | "disabled" | "unreachable" | "error".
    property string status: "unknown"
    // Last parsed reply object from `dataCommand` (null until first fetch).
    property var data: null

    readonly property bool ok: status === "ok"
    // Configured but the server isn't serving — the graceful-notice case.
    readonly property bool degraded: status === "unreachable" || status === "error"
    // Not configured — the widget should collapse entirely.
    readonly property bool disabled: status === "disabled"

    // Fired on any status or data change so hosts can react (re-anchor focus,
    // scroll into view, etc.).
    signal updated

    // Pure status setter — set + signal only. Crucially it does NOT kick off a
    // fetch: doing so from inside `dataReq.onResponseReceived` would re-enter the
    // same socket mid-reply and race into a spurious `requestFailed`, latching
    // the status back to `unreachable`. The recovery-refetch lives only on the
    // health-event path below (broadcast events carry no data payload).
    function _applyStatus(next) {
        if (next === mon.status)
            return;
        mon.status = next;
        mon.updated();
    }

    function refresh() {
        if (mon.dataCommand !== "")
            dataReq.request(mon.dataCommand);
    }

    // Broadcast-mode status source: the daemon's global health events.
    SocketClient {
        id: healthSub
        subscribe: true
        onLineReceived: line => {
            if (!line.startsWith("health:"))
                return;
            try {
                let o = JSON.parse(line.slice(7));
                if (o.service === mon.healthKey && typeof o.status === "string") {
                    let wasOk = mon.ok;
                    mon._applyStatus(o.status);
                    // Recovered via a (payload-less) health event → fetch fresh
                    // data now rather than waiting for the next poll tick. Safe
                    // here: not re-entrant with a data reply.
                    if (o.status === "ok" && !wasOk && mon.dataCommand !== "")
                        dataReq.request(mon.dataCommand);
                }
            } catch (e)
            // Malformed event — ignore, keep last status.
            {}
        }
    }

    // Data fetch (both modes) and poll-mode status source.
    SocketClient {
        id: dataReq
        onResponseReceived: line => {
            try {
                let o = JSON.parse(line);
                mon.data = o;
                // A reply that carries its own status keeps `status` fresh even
                // without a broadcast event (poll mode relies on this; broadcast
                // mode just stays in agreement).
                if (typeof o.status === "string")
                    mon._applyStatus(o.status);
                mon.updated();
            } catch (e) {
                console.log("ServiceMonitor[" + mon.healthKey + "]: bad reply: " + e);
            }
        }
        // Daemon down / connect failure: treat as unreachable so the widget
        // shows the graceful notice rather than stale-but-confident data.
        onRequestFailed: mon._applyStatus("unreachable")
    }

    // Periodic data refresh / poll. The reply carries `status`, so this poll is
    // the self-healing floor in BOTH modes: even if a broadcast event is missed
    // (the daemon emits health on change, so a late subscriber can sit on a
    // stale/wrong status) the next poll re-fetches and corrects within one
    // interval. The broadcast subscription is the fast-path on top, not the only
    // source. Skip only when the service is unconfigured (`disabled`) — there's
    // nothing to fetch and the poller will broadcast if config ever appears.
    Timer {
        interval: mon.dataIntervalMs
        running: mon.active && mon.dataCommand !== ""
        repeat: true
        // Fire when the timer (re)starts so becoming active polls immediately,
        // rather than waiting a full interval.
        triggeredOnStart: true
        onTriggered: {
            if (mon.disabled)
                return;
            dataReq.request(mon.dataCommand);
        }
    }

    Component.onCompleted: {
        if (mon.healthKey !== "")
            healthSub.start();
        // Prime once on start so the first paint is correct without waiting for
        // the next broadcast/poll tick (the daemon emits health on change, so a
        // late subscriber would otherwise sit at "unknown"). Skip the prime for
        // an inactive poll-mode monitor so we don't poll a known-offline host.
        if (mon.dataCommand !== "" && (mon.healthKey !== "" || mon.active))
            dataReq.request(mon.dataCommand);
    }

    Component.onDestruction: {
        if (mon.healthKey !== "")
            healthSub.stop();
    }
}
