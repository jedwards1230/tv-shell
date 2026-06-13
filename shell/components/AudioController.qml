import QtQuick
import Quickshell.Io

// Shared PipeWire/wpctl audio controller. Owns volume/mute/sink state and the
// five wpctl Process blocks shared by VolumeOverlay, SessionQAM, and AudioSettings.
//
// Lifted verbatim from VolumeOverlay/SessionQAM to guarantee the command strings
// and parsing match exactly. AudioSettings adds node-name persistence on top of
// sinkSwitched — it keeps its own readNodeName/reapplySink/getFormat Processes
// and connects them to this signal.
//
// Usage:
//   AudioController { id: audioCtl }
//   onVolumeChanged / onMutedChanged: bind UI
//   audioCtl.refresh()          — (re)read volume + sinks
//   audioCtl.setVolumeLevel(n)  — set volume to n%
//   audioCtl.toggleMuteState()  — toggle mute
//   audioCtl.setDefaultSinkById(id) — switch output sink
Item {
    id: audio

    property int volume: 50
    property bool muted: false
    property var sinks: []
    property int defaultSinkIndex: -1

    // (Re)read volume and sink list.
    function refresh() {
        getVolume.running = true;
        listSinks.running = true;
    }

    // Return the display name of the current default sink.
    function currentSinkName() {
        if (audio.defaultSinkIndex >= 0 && audio.defaultSinkIndex < audio.sinks.length)
            return audio.sinks[audio.defaultSinkIndex].name;
        return "No output device";
    }

    // Clamp and apply a volume percentage.
    function setVolumeLevel(pct) {
        audio.volume = Math.max(0, Math.min(100, pct));
        setVolume.level = audio.volume + "%";
        setVolume.running = true;
    }

    // Toggle mute.
    function toggleMuteState() {
        toggleMute.running = true;
    }

    // Switch the default sink by numeric wpctl id.
    function setDefaultSinkById(sinkId) {
        if (setDefaultSink.running)
            return;
        setDefaultSink.sinkId = sinkId;
        setDefaultSink.running = true;
    }

    // Emitted after the sink list refreshes so cursor-tracking consumers can
    // sync their UI cursor to the new defaultSinkIndex.
    signal sinkCursorSync(int defaultIndex)

    // Emitted after setDefaultSink exits, carrying the numeric sink id that was
    // applied. AudioSettings connects this to kick readNodeName + getFormat.
    signal sinkSwitched(int sinkId)

    // --- wpctl processes ---

    Process {
        id: getVolume
        command: ["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"]
        stdout: SplitParser {
            onRead: line => {
                let parts = line.trim().split(" ");
                if (parts.length >= 2)
                    audio.volume = Math.round(parseFloat(parts[1]) * 100);
                audio.muted = line.indexOf("[MUTED]") >= 0;
            }
        }
    }

    Process {
        id: setVolume
        property string level: "50%"
        command: ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", level]
        onExited: getVolume.running = true
    }

    Process {
        id: toggleMute
        command: ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]
        onExited: getVolume.running = true
    }

    Process {
        id: listSinks
        command: ["bash", "-c", "wpctl status | sed -n '/Audio/,/Video/p' | sed -n '/Sinks:/,/Sources:/p' | grep -v 'Sinks:\\|Sources:\\|^$'"]
        stdout: SplitParser {
            property var collected: []
            onRead: line => {
                let cleaned = line.replace(/[│├└─┐┘┌┬┴┤┼]/g, " ");
                let isDefault = cleaned.indexOf("*") >= 0;
                let match = cleaned.match(/\*?\s*(\d+)\.\s+(.+?)(?:\s+\[vol:.+\])?\s*$/);
                if (match) {
                    let entry = {
                        id: parseInt(match[1]),
                        name: match[2].trim(),
                        isDefault: isDefault
                    };
                    collected.push(entry);
                    if (isDefault)
                        audio.defaultSinkIndex = collected.length - 1;
                }
            }
        }
        onExited: {
            audio.sinks = listSinks.stdout.collected;
            listSinks.stdout.collected = [];
            if (audio.defaultSinkIndex >= 0)
                audio.sinkCursorSync(audio.defaultSinkIndex);
        }
    }

    Process {
        id: setDefaultSink
        property int sinkId: 0
        command: ["wpctl", "set-default", String(sinkId)]
        onExited: {
            listSinks.running = true;
            refreshTimer.start();
            audio.sinkSwitched(sinkId);
        }
    }

    Timer {
        id: refreshTimer
        interval: 500
        onTriggered: getVolume.running = true
    }
}
