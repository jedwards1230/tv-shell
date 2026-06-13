import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import "lib"

FocusScope {
    id: root
    implicitHeight: audioMainCol.implicitHeight + 2 * Theme.padding

    property int volume: 50
    property bool muted: false
    property var sinks: []
    property int defaultSinkIndex: -1
    property string formatInfo: "Unavailable"

    // Output card profiles (#234): the available output profiles of every audio
    // card (stereo / surround 5.1 / 7.1 / …). PipeWire auto-selects the highest-
    // *priority* available profile, which is always stereo — so a 5.1-capable AVR
    // over HDMI silently lands on a 2-channel sink. Surfacing the profile here lets
    // the user switch the card to Digital Surround 5.1 and make 5.1 active.
    property var cardProfiles: []
    property int currentProfileIndex: -1

    // Guard: re-apply persisted default only once per page-load
    property bool _reapplied: false

    // --- Processes ---

    Process {
        id: getVolume
        command: ["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"]
        stdout: SplitParser {
            onRead: line => {
                // Output: "Volume: 0.50" or "Volume: 0.50 [MUTED]"
                let parts = line.trim().split(" ");
                if (parts.length >= 2) {
                    root.volume = Math.round(parseFloat(parts[1]) * 100);
                }
                root.muted = line.indexOf("[MUTED]") >= 0;
            }
        }
    }

    Process {
        id: setVolume
        property string level: "50%"
        command: ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", level]
        onExited: {
            getVolume.running = true;
        }
    }

    Process {
        id: toggleMute
        command: ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]
        onExited: {
            getVolume.running = true;
        }
    }

    Process {
        id: listSinks
        command: ["bash", "-c", "wpctl status | sed -n '/Audio/,/Video/p' | sed -n '/Sinks:/,/Sources:/p' | grep -v 'Sinks:\\|Sources:\\|^$'"]
        stdout: SplitParser {
            property var collected: []
            onRead: line => {
                // Lines like: " │      46. <AVR name>  [vol: 1.00]"
                //             " │  *   86. Radeon HD Audio   [vol: 1.00]"
                // Strip box-drawing chars and leading whitespace
                let cleaned = line.replace(/[│├└─┐┘┌┬┴┤┼]/g, " ");
                let isDefault = cleaned.indexOf("*") >= 0;
                // Extract id and name from "  *   86. Some Name  [vol: 1.00]"
                let match = cleaned.match(/\*?\s*(\d+)\.\s+(.+?)(?:\s+\[vol:.+\])?\s*$/);
                if (match) {
                    let entry = {
                        id: parseInt(match[1]),
                        name: match[2].trim(),
                        isDefault: isDefault
                    };
                    collected.push(entry);
                    if (isDefault)
                        root.defaultSinkIndex = collected.length - 1;
                }
            }
        }
        onExited: {
            root.sinks = listSinks.stdout.collected;
            listSinks.stdout.collected = [];
            // Re-apply persisted default once on first populate
            if (!root._reapplied && SettingsStore.defaultSink !== "") {
                root._reapplied = true;
                reapplySink.wantName = SettingsStore.defaultSink;
                reapplySink.running = true;
            }
            // Refresh format info after sink list updates
            getFormat.running = true;
        }
    }

    // Live-switch the default sink by numeric id (volatile — changes across reboots)
    Process {
        id: setDefaultSink
        property int sinkId: 0
        command: ["wpctl", "set-default", String(sinkId)]
        onExited: {
            // After switching, read the node.name of the new default to persist it
            readNodeName.pendingSinkId = sinkId;
            readNodeName.running = true;
            listSinks.running = true;
            refreshTimer.start();
        }
    }

    // Read the stable node.name from wpctl inspect for a given numeric id,
    // then persist it via SettingsStore so it survives reboots.
    Process {
        id: readNodeName
        property int pendingSinkId: 0
        command: ["bash", "-c", "wpctl inspect " + pendingSinkId + " | sed -n 's/.*node\\.name = \"\\(.*\\)\".*/\\1/p'"]
        stdout: SplitParser {
            onRead: line => {
                let nodeName = line.trim();
                if (nodeName !== "")
                    SettingsStore.setDefaultSink(nodeName);
            }
        }
    }

    // Re-apply persisted default on boot: resolve node.name -> numeric id via
    // pw-dump (prefer jq if available) then call wpctl set-default.
    //
    // Resolution strategy: pw-dump | jq is robust; if jq is absent we fall back
    // to grepping wpctl status for the name (good enough for nearly all setups).
    Process {
        id: reapplySink
        property string wantName: ""
        // Pass sink name via env variable so it cannot inject shell commands
        // regardless of its content. (#131 shell injection fix)
        environment: ({
                "GAME_SHELL_SINK": wantName
            })
        command: ["bash", "-c", "[ -z \"$GAME_SHELL_SINK\" ] && exit 0; " + "if command -v jq >/dev/null 2>&1; then " + "  id=$(pw-dump 2>/dev/null | jq -r --arg n \"$GAME_SHELL_SINK\" " + "    '.[] | select(.info.props[\"node.name\"]==$n) | .id' | head -1); " + "else " + "  id=$(wpctl status 2>/dev/null | grep -F \"$GAME_SHELL_SINK\" | grep -oE '[0-9]+' | head -1); " + "fi; " + "[ -n \"$id\" ] && wpctl set-default \"$id\" || true"]
        onExited: {
            // Refresh the list so UI reflects the re-applied default
            listSinks.running = true;
        }
    }

    Timer {
        id: refreshTimer
        interval: 500
        onTriggered: {
            getVolume.running = true;
            getFormat.running = true;
        }
    }

    // Sample-rate / format display — read from the current default sink
    // Parsed into e.g. "48000 Hz · S16LE · 6 ch"; falls back to "Unavailable"
    Process {
        id: getFormat
        command: ["bash", "-c", "wpctl inspect @DEFAULT_AUDIO_SINK@ 2>/dev/null | grep -E 'audio\\.rate|audio\\.format|audio\\.channels'"]
        stdout: SplitParser {
            property string rate: ""
            property string fmt: ""
            property string channels: ""
            onRead: line => {
                let m;
                m = line.match(/audio\.rate\s*=\s*"?(\d+)"?/);
                if (m)
                    getFormat.stdout.rate = m[1];
                m = line.match(/audio\.format\s*=\s*"?([A-Za-z0-9_]+)"?/);
                if (m)
                    getFormat.stdout.fmt = m[1];
                m = line.match(/audio\.channels\s*=\s*"?(\d+)"?/);
                if (m)
                    getFormat.stdout.channels = m[1];
            }
        }
        onExited: {
            let r = getFormat.stdout.rate;
            let f = getFormat.stdout.fmt;
            let c = getFormat.stdout.channels;
            getFormat.stdout.rate = "";
            getFormat.stdout.fmt = "";
            getFormat.stdout.channels = "";
            if (r !== "" || f !== "" || c !== "") {
                let parts = [];
                if (r !== "")
                    parts.push(r + " Hz");
                if (f !== "")
                    parts.push(f);
                if (c !== "")
                    parts.push(c + " ch");
                root.formatInfo = parts.join(" · ");
            } else {
                root.formatInfo = "Unavailable";
            }
        }
    }

    // Enumerate every card's AVAILABLE output profiles (#234). Emits one
    // tab-separated line per profile: `<cardName>\t<profileName>\t<active>\t<desc>`.
    // Only output-bearing profiles on connected ports (`available: yes`) are
    // listed, so the dropdown shows e.g. "Digital Stereo (HDMI 2)" alongside
    // "Digital Surround 5.1 (HDMI 2)".
    Process {
        id: listProfiles
        command: ["bash", "-c", "pactl list cards | awk '" + "/^Card #/ { flush(); card=\"\"; ap=\"\"; n=0; inprof=0 } " + "/^[ \\t]*Name:/ { card=$2 } " + "/^[ \\t]*Profiles:/ { inprof=1; next } " + "/^[ \\t]*Active Profile:/ { ap=$3; inprof=0; flush() } " + "inprof && /available: yes/ { line=$0; sub(/^[ \\t]+/,\"\",line); ci=index(line,\": \"); if(ci<1) next; pname=substr(line,1,ci-1); if(pname !~ /^output:/) next; rest=substr(line,ci+2); si=index(rest,\" (sinks:\"); if(si>0) desc=substr(rest,1,si-1); else desc=rest; buf[n]=pname \"|\" desc; n++ } " + "function flush(  i,a){ for(i=0;i<n;i++){ split(buf[i],a,\"|\"); print card \"\\t\" a[1] \"\\t\" ((a[1]==ap)?\"1\":\"0\") \"\\t\" a[2] } n=0 } " + "END { flush() }'"]
        stdout: SplitParser {
            property var collected: []
            onRead: line => {
                let parts = line.split("\t");
                if (parts.length < 4)
                    return;
                let entry = {
                    card: parts[0],
                    profile: parts[1],
                    active: parts[2] === "1",
                    desc: parts[3]
                };
                collected.push(entry);
                if (entry.active)
                    root.currentProfileIndex = collected.length - 1;
            }
        }
        onExited: {
            root.cardProfiles = listProfiles.stdout.collected;
            listProfiles.stdout.collected = [];
        }
    }

    // Apply a card profile (#234), then make that card's resulting sink the
    // default so the surround channels become active and the Speaker Test (which
    // targets @DEFAULT_AUDIO_SINK@) exercises all 6. Card/profile are passed via
    // env so their content can never inject shell commands.
    Process {
        id: setCardProfile
        property string cardName: ""
        property string profileName: ""
        environment: ({
                "GS_CARD": cardName,
                "GS_PROFILE": profileName
            })
        command: ["bash", "-c", "[ -z \"$GS_CARD\" ] && exit 0; " + "pactl set-card-profile \"$GS_CARD\" \"$GS_PROFILE\" || exit 0; " + "sink=$(pactl list sinks | awk -v c=\"$GS_CARD\" '/^[ \\t]*Name:/{n=$2} /device.name = /{ gsub(/\"/,\"\"); if($3==c) print n }' | head -1); " + "[ -n \"$sink\" ] && pactl set-default-sink \"$sink\" || true"]
        onExited: {
            // Refresh everything: new default sink, format, and active profile.
            listSinks.running = true;
            getFormat.running = true;
            listProfiles.running = true;
            refreshTimer.start();
        }
    }

    // Shared process for 5.1 channel test tones — single-shot so it exits
    // promptly and never hangs audio.
    //
    // ALSA 6-channel speaker index map (standard 5.1 order):
    //   FL=1, FR=2, Rear L=3, Rear R=4, Center=5, LFE=6
    Process {
        id: testTone
        property var pendingCmd: []
        command: pendingCmd
    }

    Component.onCompleted: {
        getVolume.running = true;
        listSinks.running = true;
        getFormat.running = true;
        listProfiles.running = true;
    }

    // Refresh when section becomes visible. Do NOT grab focus here — focus
    // entry is driven explicitly by SettingsPanel via focusFirst() on Right,
    // so swapping to this page with A leaves focus on the sidebar.
    onVisibleChanged: {
        if (visible) {
            root._reapplied = false;
            getVolume.running = true;
            listSinks.running = true;
            getFormat.running = true;
            listProfiles.running = true;
        }
    }

    // First interactive element is the real key-handling FocusScope, not the
    // bare volumeRow layout (which has no Keys handlers to receive focus).
    function focusFirst() {
        volDownScope.forceActiveFocus();
    }

    ColumnLayout {
        id: audioMainCol
        anchors.fill: parent
        anchors.margins: Theme.padding
        spacing: 32

        // Volume control
        SectionHeader {
            text: "Volume"
        }

        RowLayout {
            id: volumeRow
            Layout.fillWidth: true
            spacing: 24

            FocusScope {
                id: volDownScope
                width: volDownBtn.width
                height: volDownBtn.height
                activeFocusOnTab: true

                KeyNavigation.right: volUpScope
                KeyNavigation.down: muteScope

                SettingsButton {
                    id: volDownBtn
                    text: "  -  "
                    focus: parent.activeFocus
                    anchors.fill: parent

                    onActivated: {
                        root.volume = Math.max(0, root.volume - 5);
                        setVolume.level = root.volume + "%";
                        setVolume.running = true;
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            volDownScope.forceActiveFocus();
                            volDownBtn.activated();
                        }
                    }
                }
            }

            // Volume bar
            Rectangle {
                Layout.fillWidth: true
                height: 56
                radius: 28
                color: Theme.surfaceHover

                Rectangle {
                    width: parent.width * (root.volume / 100)
                    height: parent.height
                    radius: 28
                    color: root.muted ? Theme.textSecondary : (Theme.darkMode ? Theme.ember : Theme.navy)

                    Behavior on width {
                        NumberAnimation {
                            duration: 100
                        }
                    }
                }

                Text {
                    anchors.centerIn: parent
                    text: root.muted ? "MUTED" : root.volume + "%"
                    font.pixelSize: Theme.fontBody
                    font.bold: true
                    color: root.volume > 40 ? Theme.textOnDark : Theme.textPrimary
                }
            }

            FocusScope {
                id: volUpScope
                width: volUpBtn.width
                height: volUpBtn.height
                activeFocusOnTab: true

                KeyNavigation.left: volDownScope
                KeyNavigation.down: muteScope

                SettingsButton {
                    id: volUpBtn
                    text: "  +  "
                    focus: parent.activeFocus
                    anchors.fill: parent

                    onActivated: {
                        root.volume = Math.min(100, root.volume + 5);
                        setVolume.level = root.volume + "%";
                        setVolume.running = true;
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            volUpScope.forceActiveFocus();
                            volUpBtn.activated();
                        }
                    }
                }
            }
        }

        FocusScope {
            id: muteScope
            width: muteBtn.width
            height: muteBtn.height
            activeFocusOnTab: true

            KeyNavigation.up: volDownScope
            KeyNavigation.down: sinkDropdownScope

            SettingsButton {
                id: muteBtn
                text: root.muted ? "Unmute" : "Mute"
                focus: parent.activeFocus
                anchors.fill: parent

                onActivated: toggleMute.running = true

                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        muteScope.forceActiveFocus();
                        muteBtn.activated();
                    }
                }
            }
        }

        // Output device dropdown
        SectionHeader {
            text: "Output Device"
        }

        SettingsDropdown {
            id: sinkDropdownScope
            model: root.sinks
            displayText: root.defaultSinkIndex >= 0 && root.defaultSinkIndex < root.sinks.length ? root.sinks[root.defaultSinkIndex].name : "No output device"
            maxHeight: 500
            rowHeight: 72
            isCurrentItem: function (item) {
                return item.isDefault;
            }
            itemLabel: function (item) {
                return item.name;
            }
            onItemSelected: function (item) {
                setDefaultSink.sinkId = item.id;
                setDefaultSink.running = true;
            }

            KeyNavigation.up: muteScope
            KeyNavigation.down: profileDropdownScope
        }

        // Output profile (surround) — #234. Lets the user switch a card to its
        // 5.1/7.1 profile; PipeWire otherwise always defaults to stereo.
        SectionHeader {
            text: "Output Profile (Surround)"
        }

        SettingsDropdown {
            id: profileDropdownScope
            model: root.cardProfiles
            displayText: root.currentProfileIndex >= 0 && root.currentProfileIndex < root.cardProfiles.length ? root.cardProfiles[root.currentProfileIndex].desc : "Stereo"
            maxHeight: 500
            rowHeight: 72
            isCurrentItem: function (item) {
                return item.active;
            }
            itemLabel: function (item) {
                return item.desc;
            }
            onItemSelected: function (item) {
                setCardProfile.cardName = item.card;
                setCardProfile.profileName = item.profile;
                setCardProfile.running = true;
            }

            KeyNavigation.up: sinkDropdownScope
            KeyNavigation.down: testToneFlScope
        }

        Text {
            text: "Switch HDMI/analog output between stereo and surround (5.1 / 7.1) when the receiver supports it."
            font.pixelSize: Theme.fontHint
            color: Theme.textMuted
        }

        // ---------------------------------------------------------------
        // Speaker Test (5.1)
        //
        // ALSA 6-channel index map (document for maintenance):
        //   FL=1 (Front Left), FR=2 (Front Right),
        //   Rear L=3, Rear R=4, Center=5, LFE=6
        // single-shot: -l 1 ensures speaker-test exits after one pass.
        // ---------------------------------------------------------------
        SectionHeader {
            text: "Speaker Test (5.1)"
        }

        Text {
            text: "Sink: " + sinkDropdownScope.displayText
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
        }

        // Row 1: FL, FR, Center
        RowLayout {
            Layout.fillWidth: true
            spacing: 16

            FocusScope {
                id: testToneFlScope
                width: testToneFlBtn.width
                height: testToneFlBtn.height
                activeFocusOnTab: true

                KeyNavigation.up: profileDropdownScope
                KeyNavigation.right: testToneFrScope
                KeyNavigation.down: testToneRlScope

                SettingsButton {
                    id: testToneFlBtn
                    text: "Front L"
                    focus: parent.activeFocus
                    anchors.fill: parent
                    onActivated: {
                        // FL = channel 1
                        testTone.pendingCmd = ["speaker-test", "-D", "default", "-c", "6", "-t", "sine", "-f", "440", "-l", "1", "-s", "1"];
                        testTone.running = true;
                    }
                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            testToneFlScope.forceActiveFocus();
                            testToneFlBtn.activated();
                        }
                    }
                }
            }

            FocusScope {
                id: testToneFrScope
                width: testToneFrBtn.width
                height: testToneFrBtn.height
                activeFocusOnTab: true

                KeyNavigation.up: profileDropdownScope
                KeyNavigation.left: testToneFlScope
                KeyNavigation.right: testToneCScope
                KeyNavigation.down: testToneRrScope

                SettingsButton {
                    id: testToneFrBtn
                    text: "Front R"
                    focus: parent.activeFocus
                    anchors.fill: parent
                    onActivated: {
                        // FR = channel 2
                        testTone.pendingCmd = ["speaker-test", "-D", "default", "-c", "6", "-t", "sine", "-f", "440", "-l", "1", "-s", "2"];
                        testTone.running = true;
                    }
                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            testToneFrScope.forceActiveFocus();
                            testToneFrBtn.activated();
                        }
                    }
                }
            }

            FocusScope {
                id: testToneCScope
                width: testToneCBtn.width
                height: testToneCBtn.height
                activeFocusOnTab: true

                KeyNavigation.up: profileDropdownScope
                KeyNavigation.left: testToneFrScope
                KeyNavigation.down: testToneLfeScope

                SettingsButton {
                    id: testToneCBtn
                    text: "Center"
                    focus: parent.activeFocus
                    anchors.fill: parent
                    onActivated: {
                        // Center = channel 5
                        testTone.pendingCmd = ["speaker-test", "-D", "default", "-c", "6", "-t", "sine", "-f", "440", "-l", "1", "-s", "5"];
                        testTone.running = true;
                    }
                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            testToneCScope.forceActiveFocus();
                            testToneCBtn.activated();
                        }
                    }
                }
            }
        }

        // Row 2: Rear L, Rear R, LFE
        RowLayout {
            Layout.fillWidth: true
            spacing: 16

            FocusScope {
                id: testToneRlScope
                width: testToneRlBtn.width
                height: testToneRlBtn.height
                activeFocusOnTab: true

                KeyNavigation.up: testToneFlScope
                KeyNavigation.right: testToneRrScope
                KeyNavigation.down: testToneAllScope

                SettingsButton {
                    id: testToneRlBtn
                    text: "Rear L"
                    focus: parent.activeFocus
                    anchors.fill: parent
                    onActivated: {
                        // Rear L = channel 3
                        testTone.pendingCmd = ["speaker-test", "-D", "default", "-c", "6", "-t", "sine", "-f", "440", "-l", "1", "-s", "3"];
                        testTone.running = true;
                    }
                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            testToneRlScope.forceActiveFocus();
                            testToneRlBtn.activated();
                        }
                    }
                }
            }

            FocusScope {
                id: testToneRrScope
                width: testToneRrBtn.width
                height: testToneRrBtn.height
                activeFocusOnTab: true

                KeyNavigation.up: testToneFrScope
                KeyNavigation.left: testToneRlScope
                KeyNavigation.right: testToneLfeScope
                KeyNavigation.down: testToneAllScope

                SettingsButton {
                    id: testToneRrBtn
                    text: "Rear R"
                    focus: parent.activeFocus
                    anchors.fill: parent
                    onActivated: {
                        // Rear R = channel 4
                        testTone.pendingCmd = ["speaker-test", "-D", "default", "-c", "6", "-t", "sine", "-f", "440", "-l", "1", "-s", "4"];
                        testTone.running = true;
                    }
                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            testToneRrScope.forceActiveFocus();
                            testToneRrBtn.activated();
                        }
                    }
                }
            }

            FocusScope {
                id: testToneLfeScope
                width: testToneLfeBtn.width
                height: testToneLfeBtn.height
                activeFocusOnTab: true

                KeyNavigation.up: testToneCScope
                KeyNavigation.left: testToneRrScope
                KeyNavigation.down: testToneAllScope

                SettingsButton {
                    id: testToneLfeBtn
                    text: "LFE/Sub"
                    focus: parent.activeFocus
                    anchors.fill: parent
                    onActivated: {
                        // LFE = channel 6
                        testTone.pendingCmd = ["speaker-test", "-D", "default", "-c", "6", "-t", "sine", "-f", "440", "-l", "1", "-s", "6"];
                        testTone.running = true;
                    }
                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            testToneLfeScope.forceActiveFocus();
                            testToneLfeBtn.activated();
                        }
                    }
                }
            }
        }

        // Row 3: All channels
        FocusScope {
            id: testToneAllScope
            width: testToneAllBtn.width
            height: testToneAllBtn.height
            activeFocusOnTab: true

            KeyNavigation.up: testToneRlScope

            SettingsButton {
                id: testToneAllBtn
                text: "All channels"
                focus: parent.activeFocus
                anchors.fill: parent
                onActivated: {
                    // WAV sweep across all 6 channels, single pass
                    testTone.pendingCmd = ["speaker-test", "-D", "default", "-c", "6", "-t", "wav", "-l", "1"];
                    testTone.running = true;
                }
                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        testToneAllScope.forceActiveFocus();
                        testToneAllBtn.activated();
                    }
                }
            }
        }

        // ---------------------------------------------------------------
        // Format / sample-rate card (read-only — not in focus chain)
        // ---------------------------------------------------------------
        SectionHeader {
            text: "Format"
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: formatLabel.implicitHeight + 48
            radius: 16
            color: Theme.surface

            Text {
                id: formatLabel
                anchors.fill: parent
                anchors.margins: 24
                text: root.formatInfo
                font.pixelSize: Theme.fontSmall
                font.family: "monospace"
                color: Theme.textPrimary
                wrapMode: Text.Wrap
            }
        }

        Item {
            Layout.fillHeight: true
        }

        HintBar {
            text: "Use A to open the output device dropdown"
        }
    }
}
