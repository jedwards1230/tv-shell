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
    // Sample-rate / format only (no channel count — the grid + profile show that).
    property string formatDetail: ""

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
            // Re-scope the profile dropdown to the new default sink's card.
            listProfiles.running = true;
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
            // Negotiated channel count drives the speaker-test layout fallback.
            root.sinkChannels = (c !== "") ? (parseInt(c) || 2) : 2;
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
            // Rate/format only — channel count is conveyed by the grid + profile.
            let detail = [];
            if (r !== "")
                detail.push(r + " Hz");
            if (f !== "")
                detail.push(f);
            root.formatDetail = detail.join(" · ");
        }
    }

    // Enumerate the AVAILABLE output profiles (#234) of the card that backs the
    // CURRENT default sink. Emits one tab-separated line per profile:
    // `<cardName>\t<profileName>\t<active>\t<desc>`. Only output-bearing profiles
    // on connected ports (`available: yes`) are listed, so the dropdown shows the
    // real choices for the active output (e.g. "Digital Stereo (HDMI 2)",
    // "Digital Surround 5.1 (HDMI 2)", "Digital Surround 7.1 (HDMI 2)").
    //
    // Scoping to the default sink's card is essential: every output card carries
    // its OWN active profile, so listing all cards yields multiple `active=1`
    // rows and the dropdown would track the wrong card (e.g. show analog stereo
    // while HDMI 5.1 is really playing). `want` is the default sink's card name,
    // resolved from the sink's `device.name` property (matches the card `Name:`).
    Process {
        id: listProfiles
        command: ["bash", "-c", "def=$(pactl get-default-sink 2>/dev/null); " + "want=$(pactl list sinks 2>/dev/null | awk -v d=\"$def\" '/^[ \\t]*Name:/{n=$2} n==d && /device\\.name = /{gsub(/\"/,\"\"); print $3; exit}'); " + "pactl list cards | awk -v want=\"$want\" '" + "/^Card #/ { flush(); card=\"\"; ap=\"\"; n=0; inprof=0 } " + "/^[ \\t]*Name:/ { card=$2 } " + "/^[ \\t]*Profiles:/ { inprof=1; next } " + "/^[ \\t]*Active Profile:/ { ap=$3; inprof=0; flush() } " + "inprof && /available: yes/ { line=$0; sub(/^[ \\t]+/,\"\",line); ci=index(line,\": \"); if(ci<1) next; pname=substr(line,1,ci-1); if(pname !~ /^output:/) next; rest=substr(line,ci+2); si=index(rest,\" (sinks:\"); if(si>0) desc=substr(rest,1,si-1); else desc=rest; buf[n]=pname \"|\" desc; n++ } " + "function flush(  i,a){ if(want!=\"\" && card!=want){ n=0; return } for(i=0;i<n;i++){ split(buf[i],a,\"|\"); print card \"\\t\" a[1] \"\\t\" ((a[1]==ap)?\"1\":\"0\") \"\\t\" a[2] } n=0 } " + "END { flush() }'"]
        stdout: SplitParser {
            property var collected: []
            onRead: line => {
                // First line of a fresh run (collected was emptied onExited):
                // clear any stale selection so a re-scope can't keep an old index.
                if (collected.length === 0)
                    root.currentProfileIndex = -1;
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
        command: ["bash", "-c", "[ -z \"$GS_CARD\" ] && exit 0; " + "pactl set-card-profile \"$GS_CARD\" \"$GS_PROFILE\" || exit 0; " + "sink=$(pactl list sinks | awk -v c=\"$GS_CARD\" '/^[ \\t]*Name:/{n=$2} /device\\.name = /{ gsub(/\"/,\"\"); if($3==c) print n }' | head -1); " + "[ -n \"$sink\" ] && pactl set-default-sink \"$sink\" || true"]
        onExited: {
            // Refresh everything: new default sink, format, and active profile.
            AudioController.refresh();
            getFormat.running = true;
            listProfiles.running = true;
        }
    }

    // Speaker test channels (#234) — the visible buttons are CONDITIONAL on the
    // active output profile: a stereo sink shows Left/Right, 5.1 shows six
    // channels, 7.1 shows eight. Each channel is an independent on/off toggle:
    // press A to start a sustained 480 Hz tone on that speaker, press again to
    // stop. Multiple channels can be on at once; "All Channels" mirrors the whole
    // set. The active set is rendered as a WAV whose channel count MATCHES the
    // layout and looped via pw-play, so each button drives its real speaker with
    // no down/upmix folding (e.g. Center into L/R on a stereo sink).
    //
    // PipeWire/WAV channel index order per channel count:
    //   2: FL,FR · 4: FL,FR,RL,RR · 6: FL,FR,FC,LFE,RL,RR · 8: +SL,SR
    readonly property var channelLayouts: ({
            "2": [
                {
                    "label": "L",
                    "idx": 0
                },
                {
                    "label": "R",
                    "idx": 1
                }
            ],
            "4": [
                {
                    "label": "Front L",
                    "idx": 0
                },
                {
                    "label": "Front R",
                    "idx": 1
                },
                {
                    "label": "Rear L",
                    "idx": 2
                },
                {
                    "label": "Rear R",
                    "idx": 3
                }
            ],
            "6": [
                {
                    "label": "Front L",
                    "idx": 0
                },
                {
                    "label": "Front R",
                    "idx": 1
                },
                {
                    "label": "Center",
                    "idx": 2
                },
                {
                    "label": "LFE/Sub",
                    "idx": 3
                },
                {
                    "label": "Rear L",
                    "idx": 4
                },
                {
                    "label": "Rear R",
                    "idx": 5
                }
            ],
            "8": [
                {
                    "label": "Front L",
                    "idx": 0
                },
                {
                    "label": "Front R",
                    "idx": 1
                },
                {
                    "label": "Center",
                    "idx": 2
                },
                {
                    "label": "LFE/Sub",
                    "idx": 3
                },
                {
                    "label": "Rear L",
                    "idx": 4
                },
                {
                    "label": "Rear R",
                    "idx": 5
                },
                {
                    "label": "Side L",
                    "idx": 6
                },
                {
                    "label": "Side R",
                    "idx": 7
                }
            ]
        })

    // Negotiated channel count of the current default sink (from getFormat),
    // used as the fallback when no profile description can be parsed.
    property int sinkChannels: 2

    // How many channels to TEST. Prefer what the active profile advertises (so
    // the grid updates the instant the user picks a profile, before the sink
    // re-negotiates), falling back to the sink's negotiated channel count.
    readonly property int channelCount: {
        var desc = (root.currentProfileIndex >= 0 && root.currentProfileIndex < root.cardProfiles.length) ? root.cardProfiles[root.currentProfileIndex].desc : "";
        if (desc.indexOf("7.1") >= 0)
            return 8;
        if (desc.indexOf("5.1") >= 0)
            return 6;
        if (desc.indexOf("4.0") >= 0 || desc.indexOf("Quad") >= 0)
            return 4;
        if (desc.indexOf("Stereo") >= 0 || desc.indexOf("Mono") >= 0)
            return 2;
        if (root.sinkChannels === 8 || root.sinkChannels === 6 || root.sinkChannels === 4)
            return root.sinkChannels;
        return 2;
    }

    readonly property var channels: layoutFor(channelCount)

    // Always returns a valid layout array — read this from imperative code
    // instead of the `channels` binding, which can be transiently undefined
    // when a signal handler (onChannelCountChanged) races its first evaluation.
    function layoutFor(n) {
        return channelLayouts[String(n)] || channelLayouts["2"];
    }

    // Parallel bool array, one per visible channel (by position in `channels`).
    property var channelActive: []
    readonly property bool anyChannelActive: channelActive.indexOf(true) >= 0
    readonly property bool allChannelsActive: channelActive.length > 0 && channelActive.indexOf(false) < 0

    // Whenever the visible layout changes (profile switch), clear the active set
    // and stop any tone so we never leave a now-hidden channel "playing".
    onChannelCountChanged: resetChannels()

    function resetChannels() {
        var ch = layoutFor(channelCount);
        var a = [];
        for (var i = 0; i < ch.length; i++)
            a.push(false);
        channelActive = a;
        tonePlayer.running = false;
    }

    function toggleChannel(pos) {
        var ch = layoutFor(channelCount);
        var arr = channelActive.slice();
        while (arr.length < ch.length)
            arr.push(false);
        arr[pos] = !arr[pos];
        channelActive = arr;
        applyTones();
    }

    function setAllChannels(on) {
        var ch = layoutFor(channelCount);
        var arr = [];
        for (var i = 0; i < ch.length; i++)
            arr.push(on);
        channelActive = arr;
        applyTones();
    }

    // Push the active set to the tone player. Empty set → stop; otherwise
    // (re)start so the regenerated multi-channel WAV reflects the current set.
    function applyTones() {
        var ch = layoutFor(channelCount);
        var mask = [];
        for (var i = 0; i < ch.length; i++)
            if (channelActive[i])
                mask.push(ch[i].idx);
        tonePlayer.nch = channelCount;
        tonePlayer.mask = mask.join(",");
        if (mask.length === 0)
            tonePlayer.running = false;
        else if (tonePlayer.running)
            tonePlayer.running = false;
        else
            // onExited restarts with the new mask
            tonePlayer.running = true;       // start fresh
    }

    // Loops a ~20s steady-tone WAV containing exactly the active channels;
    // restarts on exit while any channel stays active (continuous play, and a
    // mask change reloads via a stop→onExited→start). 480 Hz × a 24000-sample
    // block tiles seamlessly (100 samples/cycle). Mask passed via env (no inject).
    Process {
        id: tonePlayer
        property string mask: "0"
        property int nch: 2
        environment: ({
                "GS_MASK": mask,
                "GS_NCH": String(nch)
            })
        // python generates the WAV then `exec`s into pw-play (same PID), so when
        // Quickshell kills this Process to change the set, pw-play dies with it —
        // no orphaned child left playing. Fixed temp path → no file accumulation.
        // The WAV channel count (GS_NCH) tracks the active layout so each masked
        // channel index lands on its real speaker.
        command: ["python3", "-c", "import wave,struct,math,os\n" + "mask=[int(x) for x in os.environ.get('GS_MASK','0').split(',') if x!='']\n" + "nch=int(os.environ.get('GS_NCH','2') or 2)\n" + "if nch<1: nch=1\n" + "sr=48000;freq=480;amp=0.5;blk=24000\n" + "block=bytearray()\n" + "for i in range(blk):\n" + " s=int(amp*32767*math.sin(2*math.pi*freq*i/sr))\n" + " fr=[0]*nch\n" + " for c in mask:\n" + "  if 0<=c<nch: fr[c]=s\n" + " block+=struct.pack('<%dh'%nch,*fr)\n" + "data=bytes(block)*40\n" + "fn=os.path.join(os.environ.get('XDG_RUNTIME_DIR','/tmp'),'game-shell-tone.wav')\n" + "w=wave.open(fn,'w');w.setnchannels(nch);w.setsampwidth(2);w.setframerate(sr)\n" + "w.writeframes(data);w.close()\n" + "os.execvp('pw-play',['pw-play','--volume','0.85',fn])\n"]
        onExited: {
            if (root.anyChannelActive)
                running = true;
        }
    }

    Component.onCompleted: {
        resetChannels();
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
            KeyNavigation.down: root.channelCount === 2 ? btn2L : (root.channelCount === 4 ? btn4FL : (root.channelCount === 8 ? btn8FL : btn6FL))
        }

        Text {
            text: "Switch HDMI/analog output between stereo and surround (5.1 / 7.1) when the receiver supports it."
            font.pixelSize: Theme.fontHint
            color: Theme.textMuted
        }

        // ---------------------------------------------------------------
        // Speaker Test — short, soft 480 Hz tones via pw-play (#234). The
        // buttons are conditional on the active output profile's channel count
        // (see channelLayouts above): stereo → L/R, 5.1 → six, 7.1 → eight. Each
        // is its own FocusScope (FocusButton) so SettingsApp's scroll-follow
        // keeps the focused control visible.
        // ---------------------------------------------------------------
        SectionHeader {
            text: "Speaker Test"
        }

        // Status caption directly under the header — the always-on-screen spot
        // (the focused button is what scrolls into view, and the header sits just
        // above it). While a tone plays it's the "it's playing" cue + how to stop
        // (the crimson button fills are the primary feedback); otherwise it shows
        // the negotiated sample-rate / format when the system reports it. Channel
        // count is omitted (grid + profile already convey it) and the line
        // collapses entirely when there's nothing to say.
        Text {
            Layout.fillWidth: true
            visible: root.anyChannelActive || root.formatDetail !== ""
            text: root.anyChannelActive ? "● Playing — press a speaker again to stop" : root.formatDetail
            font.pixelSize: Theme.fontHint
            font.bold: root.anyChannelActive
            color: root.anyChannelActive ? Theme.ember : Theme.textMuted
            wrapMode: Text.Wrap
        }

        // Spatial speaker grid — laid out to mirror the physical speaker
        // positions, reshaped per channel count (the charm from #234). Each cell
        // is its own FocusButton (FocusScope) so SettingsApp's scroll-follow
        // tracks it; the wide "All Channels" tile sits below every layout. Only
        // the block matching the active channel count is visible — the rest
        // collapse. The boundary chains (profile dropdown ↓, All ↑) select the
        // visible block via channelCount. PipeWire index map: FL=0, FR=1, FC=2,
        // LFE=3, RL=4, RR=5, SL=6, SR=7.
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 12

            // ── Stereo (2.0):  L   R ──
            RowLayout {
                Layout.fillWidth: true
                spacing: 16
                visible: root.channelCount === 2

                FocusButton {
                    id: btn2L
                    Layout.fillWidth: true
                    text: "L"
                    fillActive: root.channelActive[0] === true
                    fillColor: Theme.sidebarActive
                    onActivated: root.toggleChannel(0)
                    KeyNavigation.up: profileDropdownScope
                    KeyNavigation.right: btn2R
                    KeyNavigation.down: allChannelsBtn
                }
                FocusButton {
                    id: btn2R
                    Layout.fillWidth: true
                    text: "R"
                    fillActive: root.channelActive[1] === true
                    fillColor: Theme.sidebarActive
                    onActivated: root.toggleChannel(1)
                    KeyNavigation.up: profileDropdownScope
                    KeyNavigation.left: btn2L
                    KeyNavigation.down: allChannelsBtn
                }
            }

            // ── Quad (4.0):  FL FR / RL RR ──
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 12
                visible: root.channelCount === 4

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16
                    FocusButton {
                        id: btn4FL
                        Layout.fillWidth: true
                        text: "Front L"
                        fillActive: root.channelActive[0] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(0)
                        KeyNavigation.up: profileDropdownScope
                        KeyNavigation.right: btn4FR
                        KeyNavigation.down: btn4RL
                    }
                    FocusButton {
                        id: btn4FR
                        Layout.fillWidth: true
                        text: "Front R"
                        fillActive: root.channelActive[1] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(1)
                        KeyNavigation.up: profileDropdownScope
                        KeyNavigation.left: btn4FL
                        KeyNavigation.down: btn4RR
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16
                    FocusButton {
                        id: btn4RL
                        Layout.fillWidth: true
                        text: "Rear L"
                        fillActive: root.channelActive[2] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(2)
                        KeyNavigation.up: btn4FL
                        KeyNavigation.right: btn4RR
                        KeyNavigation.down: allChannelsBtn
                    }
                    FocusButton {
                        id: btn4RR
                        Layout.fillWidth: true
                        text: "Rear R"
                        fillActive: root.channelActive[3] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(3)
                        KeyNavigation.up: btn4FR
                        KeyNavigation.left: btn4RL
                        KeyNavigation.down: allChannelsBtn
                    }
                }
            }

            // ── 5.1:  FL  C  FR  /  RL  LFE  RR ──
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 12
                visible: root.channelCount === 6

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16
                    FocusButton {
                        id: btn6FL
                        Layout.fillWidth: true
                        text: "Front L"
                        fillActive: root.channelActive[0] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(0)
                        KeyNavigation.up: profileDropdownScope
                        KeyNavigation.right: btn6C
                        KeyNavigation.down: btn6RL
                    }
                    FocusButton {
                        id: btn6C
                        Layout.fillWidth: true
                        text: "Center"
                        fillActive: root.channelActive[2] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(2)
                        KeyNavigation.up: profileDropdownScope
                        KeyNavigation.left: btn6FL
                        KeyNavigation.right: btn6FR
                        KeyNavigation.down: btn6LFE
                    }
                    FocusButton {
                        id: btn6FR
                        Layout.fillWidth: true
                        text: "Front R"
                        fillActive: root.channelActive[1] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(1)
                        KeyNavigation.up: profileDropdownScope
                        KeyNavigation.left: btn6C
                        KeyNavigation.down: btn6RR
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16
                    FocusButton {
                        id: btn6RL
                        Layout.fillWidth: true
                        text: "Rear L"
                        fillActive: root.channelActive[4] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(4)
                        KeyNavigation.up: btn6FL
                        KeyNavigation.right: btn6LFE
                        KeyNavigation.down: allChannelsBtn
                    }
                    FocusButton {
                        id: btn6LFE
                        Layout.fillWidth: true
                        text: "LFE/Sub"
                        fillActive: root.channelActive[3] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(3)
                        KeyNavigation.up: btn6C
                        KeyNavigation.left: btn6RL
                        KeyNavigation.right: btn6RR
                        KeyNavigation.down: allChannelsBtn
                    }
                    FocusButton {
                        id: btn6RR
                        Layout.fillWidth: true
                        text: "Rear R"
                        fillActive: root.channelActive[5] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(5)
                        KeyNavigation.up: btn6FR
                        KeyNavigation.left: btn6LFE
                        KeyNavigation.down: allChannelsBtn
                    }
                }
            }

            // ── 7.1:  FL C FR / SL ▢ SR / RL LFE RR  (center of mid row blank) ──
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 12
                visible: root.channelCount === 8

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16
                    FocusButton {
                        id: btn8FL
                        Layout.fillWidth: true
                        text: "Front L"
                        fillActive: root.channelActive[0] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(0)
                        KeyNavigation.up: profileDropdownScope
                        KeyNavigation.right: btn8C
                        KeyNavigation.down: btn8SL
                    }
                    FocusButton {
                        id: btn8C
                        Layout.fillWidth: true
                        text: "Center"
                        fillActive: root.channelActive[2] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(2)
                        KeyNavigation.up: profileDropdownScope
                        KeyNavigation.left: btn8FL
                        KeyNavigation.right: btn8FR
                        KeyNavigation.down: btn8LFE
                    }
                    FocusButton {
                        id: btn8FR
                        Layout.fillWidth: true
                        text: "Front R"
                        fillActive: root.channelActive[1] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(1)
                        KeyNavigation.up: profileDropdownScope
                        KeyNavigation.left: btn8C
                        KeyNavigation.down: btn8SR
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16
                    FocusButton {
                        id: btn8SL
                        Layout.fillWidth: true
                        text: "Side L"
                        fillActive: root.channelActive[6] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(6)
                        KeyNavigation.up: btn8FL
                        KeyNavigation.right: btn8SR
                        KeyNavigation.down: btn8RL
                    }
                    // Center of the middle row is intentionally blank (no speaker).
                    Item {
                        Layout.fillWidth: true
                    }
                    FocusButton {
                        id: btn8SR
                        Layout.fillWidth: true
                        text: "Side R"
                        fillActive: root.channelActive[7] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(7)
                        KeyNavigation.up: btn8FR
                        KeyNavigation.left: btn8SL
                        KeyNavigation.down: btn8RR
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16
                    FocusButton {
                        id: btn8RL
                        Layout.fillWidth: true
                        text: "Rear L"
                        fillActive: root.channelActive[4] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(4)
                        KeyNavigation.up: btn8SL
                        KeyNavigation.right: btn8LFE
                        KeyNavigation.down: allChannelsBtn
                    }
                    FocusButton {
                        id: btn8LFE
                        Layout.fillWidth: true
                        text: "LFE/Sub"
                        fillActive: root.channelActive[3] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(3)
                        KeyNavigation.up: btn8C
                        KeyNavigation.left: btn8RL
                        KeyNavigation.right: btn8RR
                        KeyNavigation.down: allChannelsBtn
                    }
                    FocusButton {
                        id: btn8RR
                        Layout.fillWidth: true
                        text: "Rear R"
                        fillActive: root.channelActive[5] === true
                        fillColor: Theme.sidebarActive
                        onActivated: root.toggleChannel(5)
                        KeyNavigation.up: btn8SR
                        KeyNavigation.left: btn8LFE
                        KeyNavigation.down: allChannelsBtn
                    }
                }
            }

            // ── Wide "All Channels" tile, shared below every layout. Becomes
            //    "Stop All" once anything is playing so the label always states
            //    what pressing it will do. ──
            FocusButton {
                id: allChannelsBtn
                Layout.fillWidth: true
                text: root.anyChannelActive ? "Stop All" : "All Channels"
                fillActive: root.anyChannelActive
                fillColor: Theme.sidebarActive
                onActivated: root.setAllChannels(!root.anyChannelActive)
                KeyNavigation.up: root.channelCount === 2 ? btn2L : (root.channelCount === 4 ? btn4RL : (root.channelCount === 8 ? btn8RL : btn6RL))
            }
        }

        Item {
            Layout.fillHeight: true
        }

        HintBar {
            text: root.anyChannelActive ? "A: stop tone   ·   B: back" : "A: play a test tone on the focused speaker   ·   B: back"
        }
    }
}
