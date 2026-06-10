import QtQuick
import QtQuick.Layouts
import Quickshell.Io

FocusScope {
    id: root
    implicitHeight: mlMainCol.implicitHeight + 2 * Theme.padding

    property var servers: []
    property bool showAddForm: false
    property int confirmRemoveIndex: -1

    // Connection status per host: { "host": "paired" | "unpaired" | "offline" | "checking" }
    property var hostStatus: ({})
    property int _statusCheckIndex: -1

    // Pairing state
    property int pairingServerIndex: -1
    property string pairingPin: ""
    // Host being paired (kept independently of pairingServerIndex so onExited can
    // identify it after the modal closes) and a guard so the modal's success-poll
    // or a user cancel concludes the flow without onExited firing a false toast.
    property string _pairHost: ""
    property bool _pairResolved: false

    // Form fields
    property string newName: ""
    property string newHost: ""
    property string newApp: "Desktop"
    property string newResolution: "3840x2160"
    property int newFps: 120
    property bool newHdr: true
    property string newCodec: "HEVC"

    // --- Processes ---

    // Read/write the SAME resolved targets path (Paths.targetsPath) so the load
    // here, the write below, and MoonlightProvider's load never drift.
    Process {
        id: loadServers
        command: ["cat", Paths.targetsPath]
        stdout: SplitParser {
            onRead: line => {
                try {
                    root.servers = JSON.parse(line);
                } catch (e) {
                    root.servers = [];
                }
            }
        }
        onExited: root._checkAllStatuses()
    }

    // Ensure the targets file's parent dir exists before the first write — a
    // fresh install may have no ~/.config/game-shell yet, so `tee` would fail
    // with no directory. `mkdir -p` is idempotent. The dir literal is a plain
    // path arg (no shell), so no injection surface.
    Process {
        id: ensureTargetsDir
        command: ["mkdir", "-p", Paths.gameShellConfigDir]
        onExited: saveServers.running = true
    }

    Process {
        id: saveServers
        property string json: "[]"
        // Write targets.json via tee + stdin (not `bash -c` string interpolation),
        // so a server name/host containing ', \", ;, $(...), or newlines is treated
        // as literal JSON and can never break out into a shell command. The JSON is
        // a single line (JSON.stringify, no pretty-printing) so the cat+SplitParser
        // read path in loadServers / MoonlightProvider still parses it. Path is the
        // shared Paths.targetsPath (see loadServers).
        stdinEnabled: true
        command: ["tee", Paths.targetsPath]
        onStarted: {
            write(json);
            stdinEnabled = false; // close stdin -> tee writes the file and exits
        }
        // Refresh the provider's targets so the home screen reflects edits.
        onExited: StreamProviders.active.loadTargets()
    }

    // Connection status checker — runs moonlight list per host
    Process {
        id: statusChecker
        property string _host: ""
        property string _output: ""
        command: ["moonlight", "list", _host]
        stdout: SplitParser {
            onRead: line => {
                statusChecker._output += line + "\n";
            }
        }
        stderr: SplitParser {
            onRead: line => {
                statusChecker._output += line + "\n";
            }
        }
        onExited: (exitCode, exitStatus) => {
            let host = statusChecker._host;
            let output = statusChecker._output;
            statusChecker._output = "";

            let updated = root.hostStatus;
            if (output.indexOf("not been paired") >= 0)
                updated[host] = "unpaired";
            else if (exitCode === 0)
                updated[host] = "paired";
            else
                updated[host] = "offline";
            root.hostStatus = JSON.parse(JSON.stringify(updated));

            root._statusCheckIndex++;
            root._checkNextStatus();
        }
    }

    // Pairing process
    Process {
        id: pairProcess
        property string _output: ""
        // moonlight-qt's CLI `pair` does not print a PIN — the GameStream
        // handshake takes the PIN as a shared secret passed via --pin (set in
        // startPairing). We only accumulate output here for the failure message.
        stdout: SplitParser {
            onRead: line => {
                pairProcess._output += line + "\n";
            }
        }
        stderr: SplitParser {
            onRead: line => {
                pairProcess._output += line + "\n";
            }
        }
        onExited: (exitCode, exitStatus) => {
            // The success-poll or a user cancel may have already concluded the
            // flow; if so, exit quietly (a killed process returns non-zero, which
            // must NOT surface as a failure toast).
            if (root._pairResolved) {
                pairProcess._output = "";
                return;
            }
            if (exitCode === 0 && root._pairHost !== "") {
                root._markPaired(root._pairHost);
            } else {
                NotificationManager.warn("moonlight", "Pairing Failed", pairProcess._output.substring(0, 100));
                root._closePairModal();
            }
            pairProcess._output = "";
        }
    }

    // While the PIN modal is open, poll the host's pair status so the modal can
    // auto-close the moment Sunshine accepts the PIN — moonlight's `pair` does not
    // reliably exit on its own when the user completes pairing on the host side.
    Timer {
        id: pairPoll
        interval: 2000
        repeat: true
        running: root.pairingServerIndex >= 0 && !root._pairResolved
        onTriggered: {
            if (root._pairHost === "" || pairCheck.running)
                return;
            pairCheck._output = "";
            pairCheck.command = ["moonlight", "list", root._pairHost];
            pairCheck.running = true;
        }
    }

    Process {
        id: pairCheck
        property string _output: ""
        stdout: SplitParser {
            onRead: line => {
                pairCheck._output += line + "\n";
            }
        }
        stderr: SplitParser {
            onRead: line => {
                pairCheck._output += line + "\n";
            }
        }
        onExited: exitCode => {
            // `moonlight list` exits 0 and omits "not been paired" once the host
            // is paired (mirrors statusChecker) — that means pairing succeeded.
            if (root._pairResolved || root.pairingServerIndex < 0) {
                pairCheck._output = "";
                return;
            }
            if (exitCode === 0 && pairCheck._output.indexOf("not been paired") < 0)
                root._markPaired(root._pairHost);
            pairCheck._output = "";
        }
    }

    function _checkAllStatuses() {
        if (root.servers.length === 0)
            return;
        _statusCheckIndex = 0;
        let initial = {};
        for (let i = 0; i < root.servers.length; i++)
            initial[root.servers[i].host] = "checking";
        root.hostStatus = initial;
        _checkNextStatus();
    }

    function _checkNextStatus() {
        if (_statusCheckIndex >= root.servers.length) {
            _statusCheckIndex = -1;
            return;
        }
        let host = root.servers[_statusCheckIndex].host || "";
        if (host === "") {
            _statusCheckIndex++;
            _checkNextStatus();
            return;
        }
        statusChecker._host = host;
        statusChecker._output = "";
        statusChecker.running = true;
    }

    function _statusColor(host) {
        let s = root.hostStatus[host] || "";
        if (s === "paired")
            return Theme.online;
        if (s === "unpaired")
            return Theme.warning;
        if (s === "offline")
            return Theme.offline;
        return Theme.textMuted;
    }

    function _statusText(host) {
        let s = root.hostStatus[host] || "";
        if (s === "paired")
            return "Paired";
        if (s === "unpaired")
            return "Not Paired";
        if (s === "offline")
            return "Offline";
        if (s === "checking")
            return "Checking...";
        return "";
    }

    function startPairing(idx) {
        if (idx < 0 || idx >= root.servers.length)
            return;
        root.pairingServerIndex = idx;
        root._pairHost = root.servers[idx].host;
        root._pairResolved = false;
        // The shell owns the PIN: moonlight-qt's CLI `pair` does not emit one
        // (it expects the PIN as a shared secret via --pin), so generate a random
        // 4-digit PIN, show it immediately, and pass it. The user enters the same
        // PIN in Sunshine's web UI. Scraping stdout for a PIN never worked with
        // current Sunshine/Moonlight — the request just timed out.
        root.pairingPin = String(Math.floor(1000 + Math.random() * 9000));
        pairProcess._output = "";
        pairProcess.command = ["moonlight", "pair", root._pairHost, "--pin", root.pairingPin];
        pairProcess.running = true;
    }

    // Pairing succeeded (detected via poll or a clean process exit): mark the host
    // paired, notify once, and close the modal. _pairResolved makes the pair
    // process's own onExited a quiet no-op.
    function _markPaired(host) {
        if (root._pairResolved)
            return;
        root._pairResolved = true;
        let updated = root.hostStatus;
        updated[host] = "paired";
        root.hostStatus = JSON.parse(JSON.stringify(updated));
        NotificationManager.info("moonlight", "Pairing Successful", host);
        pairProcess.running = false;
        root._closePairModal();
    }

    function _closePairModal() {
        root.pairingServerIndex = -1;
        root.pairingPin = "";
        root._pairHost = "";
        // The row's action set shrinks (Pair drops once paired) — reset the
        // column so focus lands on a valid action, not a stale index.
        serverList.actionCol = 0;
        serverList.forceActiveFocus();
    }

    function cancelPairing() {
        // Mark resolved first so the process's onExited (fired by the kill below)
        // does not raise a false "Pairing Failed".
        root._pairResolved = true;
        pairProcess.running = false;
        pairProcess._output = "";
        root._closePairModal();
    }

    function persistServers() {
        saveServers.json = JSON.stringify(root.servers);
        // Re-arm stdin before every save: onStarted disables it to signal EOF to
        // tee, and that imperative assignment sticks (the declarative `stdinEnabled:
        // true` is only the initial value). Without this, the 2nd save in a session
        // writes an empty file and wipes the server list.
        saveServers.stdinEnabled = true;
        // Ensure the config dir exists, then onExited kicks off saveServers.
        ensureTargetsDir.running = true;
    }

    function addServer() {
        if (newName === "" || newHost === "")
            return;
        let entry = {
            name: newName,
            host: newHost,
            app: newApp,
            resolution: newResolution,
            fps: newFps,
            hdr: newHdr,
            codec: newCodec
        };
        let list = root.servers.slice();
        list.push(entry);
        root.servers = list;
        persistServers();
        resetForm();
    }

    function removeServer(idx) {
        let list = root.servers.slice();
        list.splice(idx, 1);
        root.servers = list;
        persistServers();
        root.confirmRemoveIndex = -1;
    }

    function resetForm() {
        newName = "";
        newHost = "";
        newApp = "Desktop";
        newResolution = "3840x2160";
        newFps = 120;
        newHdr = true;
        newCodec = "HEVC";
        showAddForm = false;
    }

    Component.onCompleted: {
        loadServers.running = true;
    }

    onVisibleChanged: {
        if (visible) {
            loadServers.running = true;
        }
    }

    // viewModeRow is a RowLayout but has Keys handlers, so it is a valid focus
    // target. Focus entry is driven by SettingsPanel via focusFirst() on Right.
    function focusFirst() {
        viewModeRow.forceActiveFocus();
    }

    ColumnLayout {
        id: mlMainCol
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 24

        // === Display Mode Toggle ===
        Text {
            text: "Display Mode"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        RowLayout {
            id: viewModeRow
            Layout.alignment: Qt.AlignLeft
            spacing: 24

            property int focusIndex: Theme.streamingViewMode === "apps" ? 1 : 0

            Keys.onLeftPressed: event => {
                // At the leftmost card, let Left bubble to SettingsPanel so it
                // returns focus to the sidebar instead of being swallowed.
                if (focusIndex > 0)
                    focusIndex--;
                else
                    event.accepted = false;
            }
            Keys.onRightPressed: {
                if (focusIndex < 1)
                    focusIndex++;
            }
            Keys.onReturnPressed: {
                Theme.setStreamingViewMode(focusIndex === 0 ? "servers" : "apps");
            }
            Keys.onDownPressed: {
                if (serverList.count > 0)
                    serverList.forceActiveFocus();
                else
                    addBtnScope.forceActiveFocus();
            }

            Repeater {
                model: [
                    {
                        id: "servers",
                        label: "Servers",
                        desc: "One card per host"
                    },
                    {
                        id: "apps",
                        label: "Apps",
                        desc: "Per-host app rows"
                    }
                ]

                Rectangle {
                    required property var modelData
                    required property int index
                    width: 340
                    height: 180
                    radius: Theme.cardRadius
                    color: Theme.surface
                    border.width: Theme.streamingViewMode === modelData.id ? 3 : 2
                    border.color: Theme.streamingViewMode === modelData.id ? Theme.focusBorder : (viewModeRow.focusIndex === index && viewModeRow.activeFocus ? Theme.focusBorder : Theme.surfaceBorder)

                    Behavior on border.color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    Rectangle {
                        anchors.fill: parent
                        radius: parent.radius
                        color: Theme.surfaceHover
                        visible: viewModeRow.focusIndex === index && viewModeRow.activeFocus && Theme.streamingViewMode !== modelData.id
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 24
                        spacing: 8

                        Item {
                            Layout.fillHeight: true
                        }

                        Text {
                            text: modelData.label
                            font.pixelSize: Theme.fontBody
                            font.bold: true
                            color: Theme.textPrimary
                            Layout.alignment: Qt.AlignHCenter
                        }

                        Text {
                            text: modelData.desc
                            font.pixelSize: Theme.fontCaption
                            color: Theme.textSecondary
                            Layout.alignment: Qt.AlignHCenter
                        }

                        Item {
                            Layout.fillHeight: true
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            viewModeRow.focusIndex = index;
                            viewModeRow.forceActiveFocus();
                            Theme.setStreamingViewMode(modelData.id);
                        }
                    }
                }
            }
        }

        // Divider
        Rectangle {
            Layout.fillWidth: true
            height: 2
            color: Theme.surfaceBorder
        }

        Text {
            text: "Streaming Servers"
            font.pixelSize: Theme.fontBody
            font.bold: true
            color: Theme.textPrimary
        }

        // Server list
        SettingsList {
            id: serverList
            // rowStride = delegate 180 + spacing 16 (#123/#139 row-count sizing).
            rowStride: 196
            maxHeight: 600
            spacing: 16
            model: root.servers
            focus: false

            // Which action in the focused row is selected (Left/Right cycles it).
            // Slot 0 = Pair when the row is unpaired; the last slot is always
            // Remove. Reset to the first action whenever the focused row changes.
            property int actionCol: 0
            // Single source of truth for which actions a row exposes, keyed on
            // status. Pair appears only when the host is reachable-but-unpaired
            // (an offline/checking host can't be paired — keeps you out of the
            // blind-launch trap). Remove is always present; it deletes the saved
            // tile, NOT the host-side pairing (moonlight has no `unpair`, so
            // un-pairing stays a Sunshine-side action). Drives button visibility,
            // highlight, and Left/Right/Return navigation so they never diverge.
            function _rowActions(host) {
                let actions = [];
                if (root.hostStatus[host] === "unpaired")
                    actions.push("pair");
                actions.push("remove");
                return actions;
            }
            onCurrentIndexChanged: actionCol = 0

            delegate: Rectangle {
                required property int index
                required property var modelData
                width: serverList.width
                height: 180
                radius: Theme.cardRadius
                color: serverList.currentIndex === index && serverList.activeFocus ? Theme.surfaceHover : Theme.surface
                border.width: 2
                border.color: Theme.surfaceBorder

                Behavior on color {
                    ColorAnimation {
                        duration: 150
                    }
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 32
                    anchors.rightMargin: 32
                    anchors.topMargin: 24
                    anchors.bottomMargin: 24
                    spacing: 32

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 8

                        RowLayout {
                            spacing: 16

                            Text {
                                text: modelData.name
                                font.pixelSize: Theme.fontBody
                                font.bold: true
                                color: Theme.textPrimary
                            }

                            Text {
                                text: modelData.host
                                font.pixelSize: Theme.fontSmall
                                color: Theme.textSecondary
                            }

                            // Status indicator
                            RowLayout {
                                spacing: 8

                                Rectangle {
                                    width: 16
                                    height: 16
                                    radius: 8
                                    color: root._statusColor(modelData.host)
                                }

                                Text {
                                    text: root._statusText(modelData.host)
                                    font.pixelSize: Theme.fontSmall
                                    color: root._statusColor(modelData.host)
                                }
                            }
                        }

                        RowLayout {
                            spacing: 24

                            Text {
                                text: modelData.app || "Desktop"
                                font.pixelSize: Theme.fontSmall
                                color: Theme.textSecondary
                                visible: text !== modelData.name
                            }

                            Text {
                                text: (modelData.resolution || "1920x1080") + " @ " + (modelData.fps || 60) + "fps"
                                font.pixelSize: Theme.fontSmall
                                color: Theme.textSecondary
                            }

                            Text {
                                text: (modelData.codec || "H.264") + (modelData.hdr ? " HDR" : "")
                                font.pixelSize: Theme.fontSmall
                                color: Theme.textSecondary
                            }
                        }
                    }

                    // Pair button — only when the host is reachable but unpaired.
                    // The FocusScope is a SIZING wrapper only: serverList holds
                    // focus (focus:false on the delegate side) and selection is
                    // driven by the button's external `highlighted` property — the
                    // scope never receives real focus. Do NOT add focus:true here;
                    // it would create a focus trap that breaks list navigation.
                    FocusScope {
                        width: pairBtn.width
                        height: pairBtn.height
                        visible: serverList._rowActions(modelData.host).indexOf("pair") >= 0

                        SettingsButton {
                            id: pairBtn
                            text: "Pair"
                            highlighted: serverList.activeFocus && serverList.currentIndex === index && serverList._rowActions(modelData.host)[serverList.actionCol] === "pair"
                            onActivated: root.startPairing(index)

                            MouseArea {
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: pairBtn.activated()
                            }
                        }
                    }

                    // Remove button. The FocusScope is a sizing wrapper only (see
                    // the Pair button above) — focus styling is the `highlighted`
                    // property, not real focus.
                    FocusScope {
                        width: removeBtn.width
                        height: removeBtn.height

                        SettingsButton {
                            id: removeBtn
                            text: "Remove"
                            highlighted: serverList.activeFocus && serverList.currentIndex === index && serverList._rowActions(modelData.host)[serverList.actionCol] === "remove"
                            onActivated: root.confirmRemoveIndex = index

                            MouseArea {
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: removeBtn.activated()
                            }
                        }
                    }
                }
            }

            // Servers launch from the home screen, not here — Return activates the
            // selected per-row action (Pair / Remove); Left/Right pick the action.
            Keys.onReturnPressed: {
                if (currentIndex < 0 || currentIndex >= root.servers.length)
                    return;
                let acts = _rowActions(root.servers[currentIndex].host);
                let a = acts[Math.min(actionCol, acts.length - 1)];
                if (a === "pair")
                    root.startPairing(currentIndex);
                else if (a === "remove")
                    root.confirmRemoveIndex = currentIndex;
            }

            Keys.onLeftPressed: {
                if (actionCol > 0)
                    actionCol--;
            }

            Keys.onRightPressed: {
                if (currentIndex < 0 || currentIndex >= root.servers.length)
                    return;
                let acts = _rowActions(root.servers[currentIndex].host);
                if (actionCol < acts.length - 1)
                    actionCol++;
            }

            Keys.onUpPressed: {
                if (currentIndex > 0)
                    currentIndex--;
                else
                    viewModeRow.forceActiveFocus();
            }

            Keys.onDownPressed: {
                if (currentIndex < root.servers.length - 1)
                    currentIndex++;
                else
                    addBtnScope.forceActiveFocus();
            }
        }

        Text {
            text: root.servers.length === 0 ? "No servers configured" : ""
            font.pixelSize: Theme.fontSmall
            color: Theme.textSecondary
            visible: text !== ""
        }

        // Add server button
        FocusScope {
            id: addBtnScope
            width: addBtn.width
            height: addBtn.height
            visible: !root.showAddForm
            activeFocusOnTab: true

            KeyNavigation.up: serverList

            SettingsButton {
                id: addBtn
                text: "Add Server"
                focus: parent.activeFocus

                onActivated: {
                    root.showAddForm = true;
                    nameInput.forceActiveFocus();
                }

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        addBtnScope.forceActiveFocus();
                        addBtn.activated();
                    }
                }
            }
        }

        // Add server form
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: visible ? addFormColumn.implicitHeight + 80 : 0
            radius: Theme.cardRadius
            color: Theme.surface
            border.width: 2
            border.color: Theme.surfaceBorder
            visible: root.showAddForm

            ColumnLayout {
                id: addFormColumn
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: 40
                spacing: 24

                Text {
                    text: "New Server"
                    font.pixelSize: Theme.fontBody
                    font.bold: true
                    color: Theme.textPrimary
                }

                // Name
                RowLayout {
                    spacing: 24

                    Text {
                        text: "Name"
                        font.pixelSize: Theme.fontSmall
                        color: Theme.textSecondary
                        Layout.preferredWidth: 160
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        height: 80
                        radius: 16
                        color: Theme.surfaceHover
                        border.width: nameInput.activeFocus ? 2 : 0
                        border.color: Theme.focusBorder

                        TextInput {
                            id: nameInput
                            anchors.fill: parent
                            anchors.leftMargin: 24
                            anchors.rightMargin: 24
                            anchors.topMargin: 20
                            anchors.bottomMargin: 20
                            text: root.newName
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textPrimary
                            clip: true
                            verticalAlignment: TextInput.AlignVCenter
                            onTextChanged: root.newName = text
                            KeyNavigation.down: hostInput
                            Keys.onEscapePressed: {
                                root.resetForm();
                                serverList.forceActiveFocus();
                            }
                        }
                    }
                }

                // Host
                RowLayout {
                    spacing: 24

                    Text {
                        text: "Host"
                        font.pixelSize: Theme.fontSmall
                        color: Theme.textSecondary
                        Layout.preferredWidth: 160
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        height: 80
                        radius: 16
                        color: Theme.surfaceHover
                        border.width: hostInput.activeFocus ? 2 : 0
                        border.color: Theme.focusBorder

                        TextInput {
                            id: hostInput
                            anchors.fill: parent
                            anchors.leftMargin: 24
                            anchors.rightMargin: 24
                            anchors.topMargin: 20
                            anchors.bottomMargin: 20
                            text: root.newHost
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textPrimary
                            clip: true
                            verticalAlignment: TextInput.AlignVCenter
                            onTextChanged: root.newHost = text
                            KeyNavigation.up: nameInput
                            KeyNavigation.down: appInput
                            Keys.onEscapePressed: {
                                root.resetForm();
                                serverList.forceActiveFocus();
                            }
                        }
                    }
                }

                // App
                RowLayout {
                    spacing: 24

                    Text {
                        text: "App"
                        font.pixelSize: Theme.fontSmall
                        color: Theme.textSecondary
                        Layout.preferredWidth: 160
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        height: 80
                        radius: 16
                        color: Theme.surfaceHover
                        border.width: appInput.activeFocus ? 2 : 0
                        border.color: Theme.focusBorder

                        TextInput {
                            id: appInput
                            anchors.fill: parent
                            anchors.leftMargin: 24
                            anchors.rightMargin: 24
                            anchors.topMargin: 20
                            anchors.bottomMargin: 20
                            text: root.newApp
                            font.pixelSize: Theme.fontSmall
                            color: Theme.textPrimary
                            clip: true
                            verticalAlignment: TextInput.AlignVCenter
                            onTextChanged: root.newApp = text
                            KeyNavigation.up: hostInput
                            Keys.onEscapePressed: {
                                root.resetForm();
                                serverList.forceActiveFocus();
                            }
                        }
                    }
                }

                // Action buttons
                RowLayout {
                    Layout.alignment: Qt.AlignRight
                    spacing: 16

                    FocusScope {
                        id: cancelScope
                        width: cancelBtn.width
                        height: cancelBtn.height

                        KeyNavigation.right: saveScope

                        SettingsButton {
                            id: cancelBtn
                            text: "Cancel"
                            focus: parent.activeFocus

                            onActivated: {
                                root.resetForm();
                                serverList.forceActiveFocus();
                            }

                            MouseArea {
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: cancelBtn.activated()
                            }
                        }
                    }

                    FocusScope {
                        id: saveScope
                        width: saveBtn.width
                        height: saveBtn.height

                        KeyNavigation.left: cancelScope

                        SettingsButton {
                            id: saveBtn
                            text: "Save"
                            focus: parent.activeFocus

                            onActivated: {
                                root.addServer();
                                serverList.forceActiveFocus();
                            }

                            MouseArea {
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: saveBtn.activated()
                            }
                        }
                    }
                }
            }
        }

        // Absorb remaining vertical space so content top-packs and the hint
        // pins to the bottom (mirrors ControllerSettings.qml).
        Item {
            Layout.fillHeight: true
        }

        // Hint
        Text {
            text: root.showAddForm ? "Esc: Cancel" : "A: Select  |  Servers are launched from Home"
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
            Layout.alignment: Qt.AlignHCenter
            visible: !root.showAddForm || root.showAddForm
        }
    }

    // Remove confirmation dialog
    Rectangle {
        anchors.fill: parent
        color: Qt.rgba(0, 0, 0, 0.7)
        visible: root.confirmRemoveIndex >= 0

        MouseArea {
            anchors.fill: parent
            onClicked: {
                root.confirmRemoveIndex = -1;
            }
        }

        Rectangle {
            anchors.centerIn: parent
            width: 800
            height: 350
            radius: 32
            color: Theme.surface

            ColumnLayout {
                anchors.centerIn: parent
                spacing: 32

                Text {
                    text: root.confirmRemoveIndex >= 0 && root.confirmRemoveIndex < root.servers.length ? "Remove \"" + root.servers[root.confirmRemoveIndex].name + "\"?" : ""
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.textPrimary
                    Layout.alignment: Qt.AlignHCenter
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    spacing: 32

                    FocusScope {
                        id: confirmRemoveYes
                        width: confirmRemoveYesBtn.width
                        height: confirmRemoveYesBtn.height

                        KeyNavigation.right: confirmRemoveNo

                        SettingsButton {
                            id: confirmRemoveYesBtn
                            text: "Remove"
                            focus: parent.activeFocus
                            onActivated: root.removeServer(root.confirmRemoveIndex)

                            MouseArea {
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: confirmRemoveYesBtn.activated()
                            }
                        }
                    }

                    FocusScope {
                        id: confirmRemoveNo
                        width: confirmRemoveNoBtn.width
                        height: confirmRemoveNoBtn.height
                        focus: root.confirmRemoveIndex >= 0

                        KeyNavigation.left: confirmRemoveYes

                        SettingsButton {
                            id: confirmRemoveNoBtn
                            text: "Cancel"
                            focus: parent.activeFocus
                            onActivated: root.confirmRemoveIndex = -1

                            MouseArea {
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: confirmRemoveNoBtn.activated()
                            }
                        }

                        Keys.onEscapePressed: {
                            root.confirmRemoveIndex = -1;
                        }
                    }
                }
            }
        }

        Keys.onEscapePressed: {
            root.confirmRemoveIndex = -1;
        }
    }

    // Pairing dialog
    Rectangle {
        anchors.fill: parent
        color: Qt.rgba(0, 0, 0, 0.7)
        visible: root.pairingServerIndex >= 0
        z: 55

        MouseArea {
            anchors.fill: parent
            onClicked: {}
        }

        Rectangle {
            anchors.centerIn: parent
            width: 900
            height: 420
            radius: 32
            color: Theme.surface

            ColumnLayout {
                anchors.centerIn: parent
                spacing: 32

                Text {
                    text: "Pairing with " + (root.pairingServerIndex >= 0 && root.pairingServerIndex < root.servers.length ? root.servers[root.pairingServerIndex].name : "")
                    font.pixelSize: Theme.fontTitle
                    font.bold: true
                    color: Theme.textPrimary
                    Layout.alignment: Qt.AlignHCenter
                }

                Text {
                    text: "Enter this PIN in the Sunshine web UI:"
                    font.pixelSize: Theme.fontBody
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignHCenter
                }

                Text {
                    visible: root.pairingPin !== ""
                    text: root.pairingPin
                    font.pixelSize: Theme.fontHero
                    font.bold: true
                    color: Theme.ember
                    Layout.alignment: Qt.AlignHCenter
                }

                Text {
                    visible: root.pairingPin !== ""
                    text: root.pairingServerIndex >= 0 && root.pairingServerIndex < root.servers.length ? "https://" + root.servers[root.pairingServerIndex].host + ":47990" : ""
                    font.pixelSize: Theme.fontSmall
                    color: Theme.textMuted
                    Layout.alignment: Qt.AlignHCenter
                }

                FocusScope {
                    id: pairCancelScope
                    width: pairCancelBtn.width
                    height: pairCancelBtn.height
                    Layout.alignment: Qt.AlignHCenter
                    focus: root.pairingServerIndex >= 0

                    SettingsButton {
                        id: pairCancelBtn
                        text: "Cancel"
                        focus: parent.activeFocus
                        onActivated: root.cancelPairing()

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: pairCancelBtn.activated()
                        }
                    }

                    Keys.onEscapePressed: root.cancelPairing()
                }
            }
        }

        Keys.onEscapePressed: root.cancelPairing()
    }
}
