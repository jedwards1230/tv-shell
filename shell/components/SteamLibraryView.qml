import QtQuick
import QtQuick.Layouts
import "lib"

// Steam-library poster view — the medium/large rendering of the unified Moonlight
// home widget (MoonlightWidget hosts this; it is NOT an independently-enableable
// widget anymore). ONE poster row with a segmented header that flips between
// "Recently Played" and "Library", fed by the daemon's `steam-library` IPC, plus
// a right-justified, status-only session indicator opposite the tabs.
//
// Selecting a card navigates Big Picture to that game on the host
// (`steam-launch <appid>` now NAVIGATES BPM, it does not launch the game) and
// then — only if no stream is already live — starts the single-target Moonlight
// stream. There is exactly one session; cards do not own a stream. The
// stream/session choreography stays in QML (HomeScreen owns it); this view only
// emits `gameSelected(appid)`.
//
// The status-only session indicator in the header reads the `streaming` flag
// from the `steam-library` reply (no separate `sunshine-status` probe).
//
// `posterScale` is set by the host so the same view renders at two sizes:
//   medium = 0.62 (smaller posters)
//   large  = 0.82 (full posters)
//
// Health-aware: a `ServiceMonitor` keyed on "steam" collapses the view when
// unconfigured/empty and shows a graceful `ServiceStatusNotice` when the host is
// down. The reply's `runningAppid` badges the card whose appid matches as
// "Playing" — source of truth is desktop-1, not which card was tapped.
//
// Focus contract (host uses these): `firstRow`/`lastRow` resolve to the first/
// last *visible* internal region (segment chips, poster row). The session
// indicator is non-focusable (status only).
ColumnLayout {
    id: root

    property Item previousRow: null
    property Item nextRow: null

    // Whether this is the *active* view of the parent MoonlightWidget (i.e. the
    // widget is enabled and its size is medium/large). The parent gates WHICH
    // view renders; this view owns its own DATA-driven visibility. The two are
    // ANDed here — the parent must NOT override `visible` directly (doing so
    // clobbers the persist-last-good binding and renders an empty zero-height
    // column when the data is good). When `viewActive` is false the view is the
    // hidden sibling; when true, `visible` reduces to the data-driven binding.
    property bool viewActive: true

    // Poster scale set by the host (medium vs large). A reflow, not a crop.
    property real posterScale: 0.62

    // Mirror of the daemon's `streaming` flag from the latest `steam-library`
    // reply (a Moonlight stream is currently live on the host). Surfaced for the
    // parent / session indicator; the library data itself is independent of it.
    property bool streaming: false

    signal escaped
    // Emitted when a card is activated; HomeScreen launches + streams the appid.
    signal gameSelected(int appid)
    signal ensureVisibleRequested(var item)

    spacing: Units.spacingMD

    // === Data (populated from `steam-library`) ===
    property var recentItems: []
    property var allItems: []
    // The host's currently-running Steam appid (or -1 when nothing is running).
    // Drives the per-card "Playing" badge.
    property int runningAppid: -1

    readonly property bool _hasRecent: recentItems.length > 0
    readonly property bool _hasAll: allItems.length > 0

    // === Segment (Recently Played vs Library) ===
    property string _segment: "recent"
    readonly property var _segmentOptions: {
        let o = [];
        if (_hasRecent)
            o.push({
                "label": "Recently Played",
                "value": "recent"
            });
        if (_hasAll)
            o.push({
                "label": "Library",
                "value": "all"
            });
        return o;
    }
    readonly property var _activeItems: _segment === "recent" ? recentItems : allItems

    readonly property bool rowFocused: posterRow.activeFocus || segmentChips.activeFocus

    // Data-driven visibility, robust against QML's short-circuit dependency
    // tracking. `_showable` is a BLOCK binding that reads all three inputs
    // unconditionally (not a `||` chain) so every dependency is registered; and
    // `visible` reads both inputs in a block too. The earlier `viewActive && (…)`
    // form went STALE: `&&` short-circuited so the data deps were never
    // registered, and the row failed to re-show when the library loaded. ANDed
    // with `viewActive` (parent decides WHICH view renders) — never set `visible`
    // from the parent, which would clobber this binding. Stay visible whenever
    // there's something to show or say:
    //   • last-good items loaded (persisted across a transient poll failure — a
    //     stream close churns a non-ok `steam-library` poll, which must NOT make
    //     the games row vanish); OR
    //   • degraded (`unreachable`/`error`) so the ServiceStatusNotice can render.
    // Hide only when truly unconfigured (`disabled`) with no data, or after a
    // successful poll returned an empty library.
    readonly property bool _showable: {
        let r = root._hasRecent;
        let a = root._hasAll;
        let deg = steamMon.degraded;
        return r || a || deg;
    }
    visible: {
        let active = root.viewActive;
        let show = root._showable;
        return active && show;
    }

    // === Home-tile focus contract ===
    readonly property var firstRow: segmentChips
    readonly property var lastRow: posterRow
    readonly property bool canFocus: visible && (root._hasRecent || root._hasAll)
    readonly property bool regionFocused: rowFocused

    function focusFirstChild() {
        if (!root.canFocus)
            return false;
        let r = root.firstRow;
        if (r && r.visible) {
            if (r.focusFirstChild)
                r.focusFirstChild();
            else
                r.forceActiveFocus();
            return true;
        }
        return false;
    }

    // === Poster geometry (reflow by posterScale) ===
    readonly property int posterW: Math.round(Theme.cardWidth * posterScale)
    readonly property int posterH: Math.round(posterW * 1.5)
    readonly property int _captionBand: Math.round(Theme.fontSmall * 1.4 + Units.spacingSM * 2)
    readonly property int steamRowHeight: posterH + _captionBand

    function refresh() {
        steamMon.refresh();
    }

    ServiceMonitor {
        id: steamMon
        healthKey: "steam"
        dataCommand: "steam-library"
        dataIntervalMs: 30000  // matches daemon service_health POLL_INTERVAL
        // ALWAYS poll — the library is available independent of any Moonlight
        // stream/session (the daemon serves `steam-library` whether or not a
        // stream is live). Nothing gates this on shellState/streaming, so a
        // stream start/stop never pauses the poll.
        active: true
        onUpdated: {
            // Adopt the library off the PAYLOAD, not the broadcast `ok` flag. A
            // `steam-library` reply carrying the library arrays is authoritative
            // even when a concurrent stream start/stop has churned the broadcast
            // `status` to a transient non-ok — that disagreement was the reason
            // a good "ok+21 games" reply could fail to render. Health events
            // (which carry no payload) and the `disabled` clear are handled
            // separately below.
            // Adopt off the reply's OWN `status` field (carried in the payload),
            // NOT the broadcast `ok` flag (a concurrent stream start/stop can churn
            // the broadcast to a transient non-ok while the reply still carries the
            // full library). Do NOT use `Array.isArray` on the payload arrays — QML
            // exposes JSON arrays as `QVariantList`, for which `Array.isArray()`
            // returns false, which silently rejected good 21-game replies.
            let d = steamMon.data;
            if (d && d.status === "ok") {
                // Guard partial replies: only overwrite a rail when the reply
                // actually carries it, so a malformed/partial "ok" can never blank
                // a good row. `runningAppid` / `streaming` always refresh on ok —
                // they are the per-game and session signals.
                if (d.recentlyPlayed !== undefined)
                    root.recentItems = d.recentlyPlayed || [];
                if (d.allGames !== undefined)
                    root.allItems = d.allGames || [];
                let ra = d.runningAppid;
                root.runningAppid = (typeof ra === "number" && ra > 0) ? ra : -1;
                root.streaming = d.streaming === true;
            } else if (d && d.status === "disabled") {
                // Unconfigured (GAME_SHELL_STEAM_URL unset): collapse for real.
                root.recentItems = [];
                root.allItems = [];
                root.runningAppid = -1;
                root.streaming = false;
            }
            // Else — a payload-less health event or a transient non-ok poll with
            // no data (`unreachable`/`error`/`unknown`, which a stream-close
            // state churn produces): KEEP the last-good lists, running-game
            // indicator, and streaming flag so the games row doesn't vanish. The
            // next good poll (or recovery refetch) refreshes them.

            // Keep the active segment on something that has content.
            if (root._segment === "recent" && !root._hasRecent && root._hasAll)
                root._segment = "all";
            else if (root._segment === "all" && !root._hasAll && root._hasRecent)
                root._segment = "recent";
            console.log("STEAMDBG status=" + (d && d.status) + " all=" + root.allItems.length + " recent=" + root.recentItems.length + " running=" + root.runningAppid + " streaming=" + root.streaming + " visible=" + root.visible);
        }
    }

    ServiceStatusNotice {
        Layout.fillWidth: true
        serviceName: "Steam"
        status: steamMon.status
    }

    // === Header: segment chips + right-justified session indicator ===
    RowLayout {
        Layout.fillWidth: true
        visible: root._hasRecent || root._hasAll
        spacing: Units.spacingXL

        FilterChips {
            id: segmentChips
            Layout.alignment: Qt.AlignVCenter
            options: root._segmentOptions
            currentIndex: {
                for (var i = 0; i < root._segmentOptions.length; i++) {
                    if (root._segmentOptions[i].value === root._segment)
                        return i;
                }
                return 0;
            }
            previousRow: root.previousRow
            nextRow: posterRow
            onFilterChanged: value => root._segment = value
            onEscaped: root.escaped()
            // Defer so the Flickable geometry is settled (the view may have been
            // hidden — Steam down — and just re-revealed) before we scroll to it.
            onActiveFocusChanged: if (activeFocus)
                Qt.callLater(() => root.ensureVisibleRequested(segmentChips))
        }

        Item {
            Layout.fillWidth: true
        }

        // Session indicator — status only (NOT focusable, NOT a pill, NOT
        // actionable). Tells the user at a glance whether a Moonlight stream is
        // live on the host, driven by the `streaming` flag from the same
        // `steam-library` reply that feeds the poster row (no separate
        // `sunshine-status` probe).
        SessionIndicator {
            Layout.alignment: Qt.AlignVCenter
            inSession: root.streaming
        }
    }

    // === The one poster row (shows the active segment) ===
    NavigableRow {
        id: posterRow
        visible: root._activeItems.length > 0
        Layout.fillWidth: true
        // Extra breathing room between the chip strip and the posters (on top of
        // the ColumnLayout spacing) so the pills don't crowd the row below.
        Layout.topMargin: Units.spacingMD
        Layout.preferredHeight: root.steamRowHeight
        keyNavigationWraps: true
        previousRow: segmentChips
        nextRow: root.nextRow
        model: root._activeItems
        onActiveFocusChanged: if (activeFocus)
            Qt.callLater(() => root.ensureVisibleRequested(posterRow))
        onActivated: {
            let it = root._activeItems[posterRow.currentIndex];
            if (it)
                root.gameSelected(it.appid);
        }
        onEscaped: root.escaped()

        delegate: SteamCard {
            required property int index
            required property var modelData
            posterWidth: root.posterW
            posterHeight: root.posterH
            showCaption: true
            title: modelData.name || ""
            art: modelData.art || ""
            headerArt: modelData.headerArt || ""
            playing: root.runningAppid > 0 && modelData.appid === root.runningAppid
            focus: index === posterRow.currentIndex
            onActivated: root.gameSelected(modelData.appid)
        }
    }
}
