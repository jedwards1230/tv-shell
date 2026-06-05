pragma Singleton
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
    }

    function removeFromHistory(id) {
        root.history = root.history.filter(function (n) {
            return n.id !== id;
        });
        if (root.unreadCount > 0)
            root.unreadCount = root.unreadCount - 1;
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
}
