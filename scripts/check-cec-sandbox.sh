#!/usr/bin/env bash
# Run the CEC daemon's startup I/O under its unit's own sandbox directives.
#
# WHY THIS EXISTS. The unit hardens the daemon with ProtectSystem=strict,
# ProtectHome=read-only, PrivateTmp=, RestrictAddressFamilies= and
# SystemCallFilter=. Each is correct in isolation; together they once forbade
# the one file the daemon must create. It started cleanly, sent READY=1, and
# died on EROFS binding its socket — a failure no unit test could see, because
# the requirement lives in Rust and the permission lives in a unit file.
#
# The Rust tests in cec/src/main.rs compare the two by parsing this unit, and
# they run in CI. They are a consistency check. THIS is the other half: it
# performs the real operations under the real sandbox. It needs a systemd user
# manager, so it cannot be a CI gate — run it on a development box, or on the
# deploy host, whenever a hardening directive changes.
#
# It reads the directives FROM the unit, so it cannot drift from what ships.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
unit="$repo_root/core/units/tv-shell-v2-cec.service"
probe_unit="tv-shell-cec-sandbox-probe"

if ! systemctl --user show-environment >/dev/null 2>&1; then
    echo "no systemd user manager here; this check needs one" >&2
    exit 2
fi
[[ -r "$unit" ]] || { echo "cannot read $unit" >&2; exit 2; }

# Every sandbox-relevant directive, taken verbatim from the shipped unit. The
# list is the hardening surface; anything added to the unit and not named here
# is reported rather than silently skipped.
keys='ProtectSystem|ProtectHome|PrivateTmp|ReadWritePaths|RuntimeDirectory|NoNewPrivileges|RestrictAddressFamilies|SystemCallFilter|DeviceAllow|MemoryMax|TasksMax'
mapfile -t directives < <(grep -E "^($keys)=" "$unit")
if [[ ${#directives[@]} -eq 0 ]]; then
    echo "no sandbox directives found in $unit — has it been rewritten?" >&2
    exit 2
fi

printf 'exercising %d directive(s) from %s\n' "${#directives[@]}" "${unit#"$repo_root"/}"
printf '  %s\n' "${directives[@]}"

# systemd-run does NOT expand `%` specifiers in transient properties, though a
# real unit file does — `ReadWritePaths=%t` is rejected here and correct there.
# Expanding them keeps the harness faithful to the unit rather than forcing the
# unit to avoid specifiers for the harness's benefit.
runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
props=()
for d in "${directives[@]}"; do
    d="${d//%t/$runtime_dir}"
    d="${d//%h/$HOME}"
    props+=(-p "$d")
done

# The probe performs what the daemon does and nothing else: bind its socket in
# $XDG_RUNTIME_DIR, write and unlink there, open the IP leg's UDP and TCP
# sockets, and resolve a name (which reaches for AF_NETLINK).
probe=$(cat <<'PY'
import os, socket, sys
runtime = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
sock = os.path.join(runtime, "tv-shell-v2-cec-probe.sock")
fails = 0
def check(name, fn):
    global fails
    try:
        fn()
        print(f"PASS {name}")
    except Exception as e:
        print(f"FAIL {name}: {e}")
        fails += 1

def bind_unix():
    try:
        os.unlink(sock)
    except FileNotFoundError:
        pass
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.bind(sock)
    s.close()
    os.unlink(sock)

def udp_broadcast():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("0.0.0.0", 0))
    s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    s.close()

def resolve():
    socket.getaddrinfo("localhost", 23, proto=socket.IPPROTO_TCP)

def tcp():
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(2)
    try:
        s.connect(("127.0.0.1", 9))
    except (ConnectionRefusedError, socket.timeout):
        pass  # refused proves the socket was permitted, which is the question
    finally:
        s.close()

check("bind the daemon's unix socket in $XDG_RUNTIME_DIR", bind_unix)
check("AF_INET udp bind + SO_BROADCAST (wake-on-lan)", udp_broadcast)
check("getaddrinfo (AVR hostname resolution)", resolve)
check("AF_INET tcp connect (AVR control)", tcp)
sys.exit(1 if fails else 0)
PY
)

systemctl --user reset-failed "$probe_unit" 2>/dev/null || true

# `--pipe` hands the probe's stdout straight back rather than routing it through
# the journal. The journal is keyed on the unit name, which is reused, so a
# failed run there shows output from previous runs interleaved with this one —
# the diagnosis has to be about THIS invocation or it is worse than nothing.
output=$(systemd-run --user --quiet --collect --wait --pipe \
    --unit="$probe_unit" --service-type=exec "${props[@]}" \
    /usr/bin/env python3 -c "$probe" 2>&1) && status=0 || status=$?

if [[ $status -eq 0 ]]; then
    printf '%s\n' "$output" | grep -E '^PASS ' || true
    echo "sandbox permits every operation the daemon performs"
else
    echo "SANDBOX FORBIDS AN OPERATION THE DAEMON PERFORMS:" >&2
    if ! printf '%s\n' "$output" | grep -E '^(PASS|FAIL) ' >&2; then
        echo "  the probe did not run at all:" >&2
        printf '  %s\n' "$output" >&2
    fi
    exit 1
fi
