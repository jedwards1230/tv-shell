pragma Singleton
import Quickshell.Io
import QtQuick

Item {
    id: root

    property var activeList: []
    property bool shellVisible: true
    property bool hasActiveError: _computeHasActiveError()

    property var history: []
    property int unreadCount: 0
    readonly property int _maxHistory: 100

    property var _queue: []
    property var _deferredQueue: []
    property int _nextId: 1
    readonly property int _maxVisible: 3

    // True once the persisted history has loaded (or failed to). notify() calls
    // before this buffer into _preloadQueue and replay afterwards, so their ids
    // are assigned after the load re-seeds _nextId (avoids a startup id collision)
    // and they survive the load replacing history (#71).
    property bool _historyLoaded: false
    property var _preloadQueue: []

    signal notificationAdded(var notification)
    signal notificationDismissed(int id)

    function _computeHasActiveError() {
        for (var i = 0; i < activeList.length; i++) {
            if (activeList[i].level === "error")
                return true;
        }
        return false;
    }

    onActiveListChanged: hasActiveError = _computeHasActiveError()

    function dismissErrors() {
        var remaining = [];
        for (var i = 0; i < root.activeList.length; i++) {
            if (root.activeList[i].level === "error")
                notificationDismissed(root.activeList[i].id);
            else
                remaining.push(root.activeList[i]);
        }
        root.activeList = remaining;
        root._queue = root._queue.filter(function (n) {
            return n.level !== "error";
        });
        root._deferredQueue = root._deferredQueue.filter(function (n) {
            return n.level !== "error";
        });
        _processQueue();
    }

    function notify(title, message, options) {
        // Until the persisted history has loaded, buffer the call: assigning an id
        // now would race the load's _nextId re-seed and the load would clobber it
        // when it replaces history. Replayed in _markHistoryLoaded() (#71).
        if (!root._historyLoaded) {
            var pq = root._preloadQueue.slice();
            pq.push({
                title: title,
                message: message,
                options: options
            });
            root._preloadQueue = pq;
            return -1;
        }
        var opts = options || Object.create(null);
        var n = Object.create(null);
        n.id = root._nextId++;
        n.title = title;
        n.message = message || "";
        n.icon = opts.icon || "";
        n.level = opts.level || "info";
        n.duration = opts.duration !== undefined ? opts.duration : _defaultDuration(opts.level || "info");
        n.source = opts.source || "system";
        n.timestamp = new Date();

        var hist = root.history.slice();
        hist.unshift(n);
        if (hist.length > root._maxHistory)
            hist = hist.slice(0, root._maxHistory);
        root.history = hist;
        root.unreadCount = root.unreadCount + 1;

        // Persist the new notification to the daemon so it survives restart.
        root._persistRecord(n);

        if (!root.shellVisible) {
            var deferred = root._deferredQueue.slice();
            deferred.push(n);
            root._deferredQueue = deferred;
            return n.id;
        }

        _enqueue(n);
        return n.id;
    }

    function dismiss(id) {
        var list = root.activeList.filter(function (n) {
            return n.id !== id;
        });
        root.activeList = list;
        notificationDismissed(id);
        _processQueue();
    }

    function dismissAll() {
        root.activeList = [];
        root._queue = [];
        root._deferredQueue = [];
    }

    function markAllRead() {
        root.unreadCount = 0;
    }

    function clearHistory() {
        root.history = [];
        root.unreadCount = 0;
        // Persist the clear so the empty state survives restart.
        root._persistAll();
    }

    function removeFromHistory(id) {
        root.history = root.history.filter(function (n) {
            return n.id !== id;
        });
        if (root.unreadCount > 0)
            root.unreadCount = root.unreadCount - 1;
        // Persist the removal so it survives restart.
        root._persistAll();
    }

    function _iconForSource(source) {
        if (source === "stream" || source === "moonlight")
            return "\u{1F4E1}";
        if (source === "controller")
            return "\u{1F3AE}";
        if (source === "network")
            return "\u{1F4F6}";
        if (source === "av")
            return "\u{1F4FA}";
        return "";
    }

    function info(source, title, message) {
        return notify(title, message || "", {
            level: "info",
            source: source,
            icon: _iconForSource(source)
        });
    }

    function warn(source, title, message) {
        return notify(title, message || "", {
            level: "warning",
            source: source,
            icon: _iconForSource(source)
        });
    }

    function error(source, title, message) {
        return notify(title, message || "", {
            level: "error",
            source: source,
            icon: _iconForSource(source)
        });
    }

    function _enqueue(n) {
        if (root.activeList.length < root._maxVisible) {
            var list = root.activeList.slice();
            list.push(n);
            root.activeList = list;
            notificationAdded(n);
        } else {
            var q = root._queue.slice();
            q.push(n);
            root._queue = q;
        }
    }

    function _processQueue() {
        if (root._queue.length > 0 && root.activeList.length < root._maxVisible) {
            var q = root._queue.slice();
            var next = q.shift();
            root._queue = q;
            _enqueue(next);
        }
    }

    function _defaultDuration(level) {
        if (level === "error")
            return 8000;
        if (level === "warning")
            return 6000;
        return 4000;
    }

    onShellVisibleChanged: {
        if (shellVisible && _deferredQueue.length > 0) {
            var deferred = _deferredQueue.slice();
            _deferredQueue = [];
            for (var i = 0; i < deferred.length; i++) {
                _enqueue(deferred[i]);
            }
        }
    }

    // -------------------------------------------------------------------------
    // Persistence — daemon IPC over native Quickshell sockets (#71).
    //
    // On boot, load the stored history from the daemon (get-notifications) and
    // re-seed the in-memory state WITHOUT replaying toasts (restored entries are
    // already read; no new toasts on restart).
    //
    // After notify(), clearHistory(), and removeFromHistory() mutate the in-memory
    // history, the writer socket pushes the change to the daemon so it survives
    // the next Quickshell restart.
    //
    // Do NOT reload after writes — in-memory is already correct and a reload
    // would race with the write.
    // -------------------------------------------------------------------------

    // Mark the persisted history as loaded (success or failure) and replay any
    // notify() calls that arrived during the async load, so they get post-reseed
    // ids and land on top of the loaded history.
    function _markHistoryLoaded() {
        if (root._historyLoaded)
            return;
        root._historyLoaded = true;
        var pending = root._preloadQueue.slice();
        root._preloadQueue = [];
        for (var i = 0; i < pending.length; i++)
            root.notify(pending[i].title, pending[i].message, pending[i].options);
    }

    SocketClient {
        id: loadNotifications
        onResponseReceived: line => {
            try {
                var entries = JSON.parse(line);
                if (Array.isArray(entries)) {
                    // Map the stored shape {id,title,message,level,source,icon,time}
                    // to the in-memory notification shape, with type guards so a
                    // malformed field can't poison numeric invariants. Entries with
                    // an invalid id are dropped (corrupt — can't be addressed by
                    // dismiss/remove anyway).
                    var loaded = entries.map(function (e) {
                        if (typeof e.id !== "number" || !Number.isInteger(e.id) || e.id <= 0) {
                            console.warn("NotificationManager: dropping persisted notification with invalid id:", e.id);
                            return null;
                        }
                        var n = Object.create(null);
                        n.id = e.id;
                        n.title = (typeof e.title === "string") ? e.title : "";
                        n.message = (typeof e.message === "string") ? e.message : "";
                        n.level = (typeof e.level === "string") ? e.level : "info";
                        n.source = (typeof e.source === "string") ? e.source : "system";
                        n.icon = (typeof e.icon === "string") ? e.icon : "";
                        n.duration = root._defaultDuration(n.level);
                        n.timestamp = new Date((typeof e.time === "number" ? e.time : 0) * 1000);
                        return n;
                    }).filter(function (n) {
                        return n !== null;
                    });
                    root.history = loaded;
                    // Restored notifications are not new — do not increment unreadCount.
                    root.unreadCount = 0;
                    // Re-seed the id counter above every loaded id (and never below
                    // its current value) so new ids can't collide with loaded ones.
                    var maxId = 0;
                    for (var i = 0; i < loaded.length; i++) {
                        if (loaded[i].id > maxId)
                            maxId = loaded[i].id;
                    }
                    root._nextId = Math.max(root._nextId, maxId + 1);
                }
            } catch (e) {
                console.warn("NotificationManager: malformed get-notifications response:", e);
            }
            root._markHistoryLoaded();
        }
        // Daemon unavailable / socket closed before a reply: degrade to an
        // in-memory-only history rather than deferring notifications forever.
        onRequestFailed: {
            console.warn("NotificationManager: failed to load notification history (daemon unavailable); continuing with empty history");
            root._markHistoryLoaded();
        }
    }

    SocketClient {
        id: notificationWriter
        // command and body are supplied dynamically by _persist*() helpers.
        // requestFailed carries no args (the socket closed before a reply); a
        // failed write means the daemon-persisted history will lag the in-memory
        // history until the next successful write — surface it rather than swallow.
        onRequestFailed: console.warn("NotificationManager: failed to persist notification history (daemon write failed); persisted state will lag until the next successful write")
    }

    // Persist a single notification addition. Include the creation time so it
    // round-trips identically to _persistAll(); the daemon falls back to its own
    // clock only when time is 0.
    function _persistRecord(n) {
        var body = JSON.stringify({
            "id": n.id,
            "title": n.title || "",
            "message": n.message || "",
            "level": n.level || "info",
            "source": n.source || "system",
            "icon": n.icon || "",
            "time": n.timestamp ? (n.timestamp.getTime() / 1000) : 0
        });
        notificationWriter.request("record-notification", body);
    }

    // Persist the full current history (used after clear/remove).
    function _persistAll() {
        var arr = root.history.map(function (n) {
            return {
                "id": n.id,
                "title": n.title || "",
                "message": n.message || "",
                "level": n.level || "info",
                "source": n.source || "system",
                "icon": n.icon || "",
                "time": n.timestamp ? (n.timestamp.getTime() / 1000) : 0
            };
        });
        notificationWriter.request("set-notifications", JSON.stringify(arr));
    }

    Component.onCompleted: {
        loadNotifications.request("get-notifications");
    }
}
