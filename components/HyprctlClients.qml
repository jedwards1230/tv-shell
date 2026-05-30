import Quickshell.Io
import QtQuick

// Queries the Hyprland client list via the input daemon's `hypr-clients` IPC
// command (see docs/IPC_PROTOCOL.md), replacing the former `hyprctl clients -j`
// shell-out. The daemon owns the Hyprland IPC connection (rust/src/hyprland.rs)
// and returns a compact single-line JSON array of {class,title,address,workspace}
// — the same fields the QML consumers read. One-shot `hyprctl dispatch` actions
// (exec/closewindow/focuswindow/fullscreen) stay shell-outs in the callers.
Item {
    id: root

    property bool running: false

    signal clientsReceived(var clients)
    signal errorOccurred(string message)

    // One-shot Unix-socket request to the input daemon (respects GAME_SHELL_SOCK,
    // falls back to the default per-UID path). The `hypr-clients` reply can be
    // large, so accumulate chunks until the first newline (the response
    // terminator); the daemon keeps the connection open after replying, so
    // reading until EOF would block until the socket timeout instead.
    readonly property string _clientsCmd: "import socket,os;s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(10);s.connect(os.environ.get('GAME_SHELL_SOCK','/run/user/'+str(os.getuid())+'/game-shell-input.sock'));s.sendall(b'hypr-clients\\n');buf=b'';exec(\"while b'\\\\n' not in buf:\\n c=s.recv(65536)\\n if not c: break\\n buf+=c\");s.close();print(buf.split(b'\\n',1)[0].decode())"

    Process {
        id: proc
        running: root.running
        command: ["python3", "-c", root._clientsCmd]
        stdout: SplitParser {
            property string buffer: ""
            onRead: line => {
                buffer += line;
            }
        }
        onExited: {
            root.running = false;
            try {
                let clients = JSON.parse(proc.stdout.buffer);
                root.clientsReceived(clients);
            } catch (e) {
                root.errorOccurred(e.toString());
            }
            proc.stdout.buffer = "";
        }
    }
}
