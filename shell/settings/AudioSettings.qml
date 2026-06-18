import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import "../components"
import "../components/lib"

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

    // --- Shared audio controller ---

    Connections {
        target: AudioController

        function onSinksChanged() {
            // Mirror sinks/defaultSinkIndex onto root for the UI.
            root.sinks = AudioController.sinks;
            root.defaultSinkIndex = AudioController.defaultSinkIndex;
            // Re-apply persisted default once on first populate.
            if (!root._reapplied && SettingsStore.defaultSink !== "") {
                root._reapplied = true;
                reapplySink.wantName = SettingsStore.defaultSink;
                reapplySink.running = true;
            }
            // Refresh format info after sink list updates.
            getFormat.running = true;
        }

        function onVolumeChanged() {
            root.volume = AudioController.volume;
        }

        function onMutedChanged() {
            root.muted = AudioController.muted;
        }

        function onSinkSwitched(sinkId) {
            // After switching, read the node.name to persist it.
            readNodeName.pendingSinkId = sinkId;
            readNodeName.running = true;
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
            AudioController.refresh();
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
            AudioController.refresh();
            getFormat.running = true;
            listProfiles.running = true;
        }
    }

    // 5.1 channel toggles (#234). Each channel is an independent on/off toggle:
    // press A to start a sustained tone on that speaker, press again to stop.
    // Multiple channels can be on at once; "All channels" mirrors the whole set
    // (and shows active when every channel is on). The active set is rendered as
    // a 6-channel WAV — a steady 480 Hz tone in each active channel — and looped
    // via pw-play. PipeWire's 6-channel order is FL,FR,FC,LFE,RL,RR →
    //   FL=0, FR=1, Center=2, LFE=3, Rear L=4, Rear R=5.
    property var channelActive: [false, false, false, false, false, false]
    readonly property var channelLabels: ["Front L", "Front R", "Center", "LFE/Sub", "Rear L", "Rear R"]
    readonly property bool anyChannelActive: channelActive.indexOf(true) >= 0
    readonly property bool allChannelsActive: channelActive.indexOf(false) < 0
    readonly property string activeLabels: {
        var names = [];
        for (var i = 0; i < 6; i++)
            if (channelActive[i])
                names.push(channelLabels[i]);
        return names.join(", ");
    }

    function toggleChannel(c) {
        var arr = channelActive.slice();
        arr[c] = !arr[c];
        channelActive = arr;
        applyTones();
    }

    function setAllChannels(on) {
        channelActive = [on, on, on, on, on, on];
        applyTones();
    }

    // Push the active set to the tone player. Empty set → stop; otherwise
    // (re)start so the regenerated multi-channel WAV reflects the current set.
    function applyTones() {
        var mask = [];
        for (var i = 0; i < 6; i++)
            if (channelActive[i])
                mask.push(i);
        tonePlayer.mask = mask.join(",");
        if (mask.length === 0)
            tonePlayer.running = false;
        else
        // onExited won't restart (none active)
        if (tonePlayer.running)
            tonePlayer.running = false;
        else
            // onExited restarts with the new mask
            tonePlayer.running = true;      // start fresh
    }

    // Loops a ~20s steady-tone WAV containing exactly the active channels;
    // restarts on exit while any channel stays active (continuous play, and a
    // mask change reloads via a stop→onExited→start). 480 Hz × a 24000-sample
    // block tiles seamlessly (100 samples/cycle). Mask passed via env (no inject).
    Process {
        id: tonePlayer
        property string mask: "0"
        environment: ({
                "GS_MASK": mask
            })
        // python generates the WAV then `exec`s into pw-play (same PID), so when
        // Quickshell kills this Process to change the set, pw-play dies with it —
        // no orphaned child left playing. Fixed temp path → no file accumulation.
        command: ["python3", "-c", "import wave,struct,math,os\n" + "mask=[int(x) for x in os.environ.get('GS_MASK','0').split(',') if x!='']\n" + "sr=48000;nch=6;freq=480;amp=0.5;blk=24000\n" + "block=bytearray()\n" + "for i in range(blk):\n" + " s=int(amp*32767*math.sin(2*math.pi*freq*i/sr))\n" + " fr=[0]*nch\n" + " for c in mask: fr[c]=s\n" + " block+=struct.pack('<%dh'%nch,*fr)\n" + "data=bytes(block)*40\n" + "fn=os.path.join(os.environ.get('XDG_RUNTIME_DIR','/tmp'),'game-shell-tone.wav')\n" + "w=wave.open(fn,'w');w.setnchannels(nch);w.setsampwidth(2);w.setframerate(sr)\n" + "w.writeframes(data);w.close()\n" + "os.execvp('pw-play',['pw-play','--volume','0.85',fn])\n"]
        onExited: {
            if (root.anyChannelActive)
                running = true;
        }
    }

    Component.onCompleted: {
        AudioController.refresh();
        getFormat.running = true;
        listProfiles.running = true;
    }

    // Refresh when section becomes visible. Do NOT grab focus here — focus
    // entry is driven explicitly by SettingsApp via focusFirst() on Right,
    // so swapping to this page with A leaves focus on the sidebar.
    onVisibleChanged: {
        if (visible) {
            root._reapplied = false;
            AudioController.refresh();
            getFormat.running = true;
            listProfiles.running = true;
        } else if (root.anyChannelActive) {
            // Don't leave tones playing after navigating away from Audio.
            root.setAllChannels(false);
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
        spacing: Units.spacingLG

        // Volume control
        SectionHeader {
            text: "Volume"
        }

        RowLayout {
            id: volumeRow
            Layout.fillWidth: true
            spacing: 24

            FocusButton {
                id: volDownScope
                KeyNavigation.right: volUpScope
                KeyNavigation.down: muteScope
                text: "  -  "
                onActivated: AudioController.setVolumeLevel(AudioController.volume - 5)
            }

            // Volume bar
            VolumeBar {
                Layout.fillWidth: true
                volume: root.volume
                muted: root.muted
                trackHeight: 56
                labelPixelSize: Theme.fontBody
            }

            FocusButton {
                id: volUpScope
                KeyNavigation.left: volDownScope
                KeyNavigation.down: muteScope
                text: "  +  "
                onActivated: AudioController.setVolumeLevel(AudioController.volume + 5)
            }
        }

        FocusButton {
            id: muteScope
            KeyNavigation.up: volDownScope
            KeyNavigation.down: sinkDropdownScope
            text: root.muted ? "Unmute" : "Mute"
            onActivated: AudioController.toggleMuteState()
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
                AudioController.setDefaultSinkById(item.id);
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
                // Persist so the surround profile is re-applied on next boot
                // (PipeWire otherwise falls back to the stereo default).
                SettingsStore.setAudioCardProfile(item.card + "|" + item.profile);
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
        // Speaker Test (5.1) — short, soft tones via pw-play (#234).
        // PipeWire 6-channel index map (FL,FR,FC,LFE,RL,RR):
        //   FL=0, FR=1, Center=2, LFE=3, Rear L=4, Rear R=5
        // ---------------------------------------------------------------
        SectionHeader {
            text: "Speaker Test (5.1)"
        }

        Text {
            text: "Sink: " + sinkDropdownScope.displayText
            font.pixelSize: Theme.fontHint
            color: Theme.textSecondary
        }

        // Row 1: Front L | Center | Front R (matches the physical speaker layout)
        RowLayout {
            Layout.fillWidth: true
            spacing: 16

            FocusButton {
                id: testToneFlScope
                KeyNavigation.up: profileDropdownScope
                KeyNavigation.right: testToneCScope
                KeyNavigation.down: testToneRlScope
                text: "Front L"
                fillActive: root.channelActive[0]
                fillColor: Theme.sidebarActive
                onActivated: root.toggleChannel(0)
            }

            FocusButton {
                id: testToneCScope
                KeyNavigation.up: profileDropdownScope
                KeyNavigation.left: testToneFlScope
                KeyNavigation.right: testToneFrScope
                KeyNavigation.down: testToneLfeScope
                text: "Center"
                fillActive: root.channelActive[2]
                fillColor: Theme.sidebarActive
                onActivated: root.toggleChannel(2)
            }

            FocusButton {
                id: testToneFrScope
                KeyNavigation.up: profileDropdownScope
                KeyNavigation.left: testToneCScope
                KeyNavigation.down: testToneRrScope
                text: "Front R"
                fillActive: root.channelActive[1]
                fillColor: Theme.sidebarActive
                onActivated: root.toggleChannel(1)
            }
        }

        // Row 2: Rear L | LFE/Sub | Rear R
        RowLayout {
            Layout.fillWidth: true
            spacing: 16

            FocusButton {
                id: testToneRlScope
                KeyNavigation.up: testToneFlScope
                KeyNavigation.right: testToneLfeScope
                KeyNavigation.down: testToneAllScope
                text: "Rear L"
                fillActive: root.channelActive[4]
                fillColor: Theme.sidebarActive
                onActivated: root.toggleChannel(4)
            }

            FocusButton {
                id: testToneLfeScope
                KeyNavigation.up: testToneCScope
                KeyNavigation.left: testToneRlScope
                KeyNavigation.right: testToneRrScope
                KeyNavigation.down: testToneAllScope
                text: "LFE/Sub"
                fillActive: root.channelActive[3]
                fillColor: Theme.sidebarActive
                onActivated: root.toggleChannel(3)
            }

            FocusButton {
                id: testToneRrScope
                KeyNavigation.up: testToneFrScope
                KeyNavigation.left: testToneLfeScope
                KeyNavigation.down: testToneAllScope
                text: "Rear R"
                fillActive: root.channelActive[5]
                fillColor: Theme.sidebarActive
                onActivated: root.toggleChannel(5)
            }
        }

        // Row 3: All channels
        FocusButton {
            id: testToneAllScope
            KeyNavigation.up: testToneLfeScope
            text: "All channels"
            fillActive: root.allChannelsActive
            fillColor: Theme.sidebarActive
            onActivated: root.setAllChannels(!root.allChannelsActive)
        }

        // Conditional "now playing" popup — appears below the grid only while a
        // tone is active, explaining how to turn it off.
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 64
            radius: Units.radiusMD
            visible: root.anyChannelActive
            color: Theme.darkMode ? Qt.rgba(Theme.ember.r, Theme.ember.g, Theme.ember.b, 0.18) : Qt.rgba(Theme.navy.r, Theme.navy.g, Theme.navy.b, 0.12)
            border.width: 2
            border.color: Theme.ember

            Text {
                anchors.fill: parent
                anchors.leftMargin: 24
                anchors.rightMargin: 24
                verticalAlignment: Text.AlignVCenter
                text: "♪  Playing: " + root.activeLabels + "   —   press a speaker (or All channels) again to turn it off"
                font.pixelSize: Theme.fontSmall
                font.bold: true
                color: Theme.textPrimary
                elide: Text.ElideRight
            }
        }

        // ---------------------------------------------------------------
        // Format / sample-rate card (read-only — not in focus chain)
        // ---------------------------------------------------------------
        SectionHeader {
            text: "Format"
        }

        ReadonlyInfoCard {
            Text {
                id: formatLabel
                width: parent.width
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
