#!/usr/bin/env python3
"""Fail if any non-test source path carries a deployment's site identity.

`docs/PRD.md` §5 locks the rule: *AV device addresses, node addresses and the
panel's deployment target are configuration, never literals in code.* The repo
is public, and site identity has been rejected from it four separate times
(#183, #287, #417, and #521's regression) — each time by hand, and each time it
came back. This is the gate that makes the rule enforceable instead of
aspirational.

## Why prose and the v2 trees are in scope now

The gate originally scanned source suffixes only, exempted `docs/`, and ran the
host-shape check over the v1 trees alone. `ced4aeb` (2026-06-06, "decouple
homelab-specific leakage from public repo") scrubbed the AVR and television
model numbers out of the tree by hand; `0a50a06` (2026-09-16, #521) put them
straight back — into `cec/README.md`, a Markdown file in a crate the gate did
not scan, under either exemption. A rule the gate cannot see is a rule that
regresses on the next documentation pass, and it did, four times.

So the exemptions are gone. `.md` is scanned like any other source, and the
host-shape check covers `cec/`, `core/`, `shell-v2/`, `docs/` and the root-level
Markdown (`CLAUDE.md`, `README.md`, `CONTRIBUTING.md`) alongside the v1 trees.
Prose is where site identity actually lands, because prose is where the
temptation to name the box you measured on lives. Measurement provenance is
still welcome — write it impersonally ("measured on the reference deployment,
2026-09-16") and keep every date and every number.

## What counts as a violation

1. **A private-network IPv4 literal** — RFC 1918 (`10/8`, `172.16/12`,
   `192.168/16`) or CGNAT (`100.64/10`, which is what a Tailscale address looks
   like). Documentation examples must use RFC 5737 TEST-NET
   (`192.0.2.x`, `198.51.100.x`, `203.0.113.x`) or a placeholder hostname.
2. **A host-shaped identifier** — `<word>-<digits>`, the near-universal shape of
   a machine name in a fleet (`htpc-1`, `desktop-2`, `nas-4`). This is a *shape*
   check, not a denylist of one site's hosts: a denylist would have to publish
   the very names it exists to keep out, and it would not catch the next
   deployment's naming at all.
3. **A MAC address**, except the documentation placeholders.
4. **An Ansible role path** — `roles/<name>/`. The deployment's provisioning
   lives in a private repo, and its role names leaked into this one the same way
   the addresses did: `roles/htpc_common/tasks/gamescope-prototype.yaml` and
   `roles/desktop-common/templates/tv-shell-host.service.j2` were cited as
   pointers in prose, a doc comment, a shipped systemd unit and an install
   script. A pointer into a repo the reader cannot open is not useful to them
   and is site identity to everyone else — describe what the role *does*
   instead, and keep the public `jedwards1230/homelab-ansible` repo name and the
   `#NNN` issue links, which are readable.

   This shape is checked everywhere, with no scope restriction: `roles/<name>/`
   has no legitimate occurrence anywhere in a Rust/QML tree, so its
   false-positive rate is zero.

   **A broader `<word>_<word>` role-name check was considered and REJECTED.**
   `[a-z]+_[a-z]+` is the shape of every snake_case identifier in a Rust tree —
   every function, field, local and module would trip it. That is the same
   argument `HOST_SHAPE_SCOPE` below already makes about Debian package names,
   and it has the same ending: a gate whose signal is buried in noise gets
   switched off, and then it catches nothing at all. A narrow rule that always
   fires beats a broad one nobody runs.

   **Known gap, accepted as the price of that trade.** Because the pattern
   requires the `roles/` prefix, it matches the *path* shape only: a bare prose
   mention of a private role by name, with no `roles/` prefix — "the Ansible
   `<role>-common` role fetches the musl binary" — is NOT detected. That is
   exactly how such a line survived a by-hand sweep in `.github/workflows/`
   (plain `rg` skips hidden directories too; re-verify with `rg --hidden
   --no-ignore`). Closing the gap needs the `<word>_<word>` shape rejected
   above, so the rule's reach stops at the path form on purpose. Bare prose
   mentions are caught by review, not by this gate.

## Why the shape check needs an allowlist

`utf-8` and `x86-64` are the same shape as `htpc-1`. `ALLOWED_TOKENS` below is
the reviewed set of non-host tokens that shape matches in this tree. Adding to
it is normal and cheap; the entry should be obviously not a machine name. If a
real host name ever needs to be added there, the fix is to remove the host name
from the source, not to widen the list.

## The `#[cfg(test)]` boundary

Test fixtures legitimately need *a* node id and *an* address, so they are
exempt — but the exemption has to be item-scoped, not file-scoped. A naive
file-level grep false-positives on `panel/src/tests.rs` (~17 fixture hits) while
missing a violation sitting above a `mod tests` block in the same file. So this
script masks exactly the `#[cfg(test)]`-gated items:

- a `#[cfg(test)] mod name;` declaration marks `name.rs` / `name/` test-only,
- a `#[cfg(test)]`-attributed inline item is masked through its closing brace.

Usage: `python3 scripts/check-site-identity.py [--verbose]` — the repo root is
resolved from this file, so it runs from anywhere. Exits 0 when clean, 1 on any
violation (printed as `path:line: what was found`).
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

# --- what we scan -----------------------------------------------------------

# Extensions worth scanning. `.md` is included: documentation is published with
# the code, a reader cannot tell prose from source when both are on GitHub, and
# every recurrence of this leak so far arrived through prose (#521 through
# `cec/README.md`). There is no documentation exemption.
SOURCE_SUFFIXES = {
    ".rs", ".qml", ".js", ".py", ".sh", ".toml", ".json", ".yaml", ".yml",
    ".md",
    ".service", ".example",  # shipped config templates count: they are read and copied
}

# The host-shape check runs over every tree that carries node identity — all the
# Rust crates (v1 *and* the v2 `core/`, `cec/`, `shell-v2/`), the QML shells, the
# shipped config examples, and `docs/`. It is still deliberately NOT run over
# workflows or install scripts, where `<word>-<digits>` is the shape of half of
# Debian's package names (`libdbus-1-dev`, `libpng16-16`) and the false-positive
# rate would swamp the signal. Addresses and MACs are checked everywhere — an
# RFC1918 literal in a workflow is a leak too.
HOST_SHAPE_SCOPE = (
    "daemon/", "panel/", "protocol/", "host/", "shell/", "shell-v2/",
    "core/", "cec/", "config/", "docs/",
)

# Root-level Markdown (CLAUDE.md, README.md, CONTRIBUTING.md) has no directory
# prefix to match on, and it is exactly where an architecture overview names the
# box it was written on. It is in the host-shape scope too.
ROOT_DOC_SUFFIXES = (".md",)


def in_host_shape_scope(rel: str) -> bool:
    if rel.startswith(HOST_SHAPE_SCOPE):
        return True
    return "/" not in rel and rel.endswith(ROOT_DOC_SUFFIXES)

# Directory names that make everything beneath them test material — as a path
# SEGMENT, so Cargo's per-crate `host/tests/` is caught alongside the top-level
# `tests/`.
TEST_DIRS = {"tests", "dev"}
TEST_FILE_PREFIXES = ("tst_",)

# Vendored third-party assets. Not ours to edit, and a minified bundle is a
# token-shape blender.
VENDORED = ("panel/assets/",)

# This script names the patterns it forbids, so it would flag itself.
SELF = "scripts/check-site-identity.py"

# --- what we forbid ---------------------------------------------------------

PRIVATE_IPV4 = re.compile(
    r"\b(?:"
    r"10\.\d{1,3}\.\d{1,3}\.\d{1,3}"
    r"|172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3}"
    r"|192\.168\.\d{1,3}\.\d{1,3}"
    r"|100\.(?:6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.\d{1,3}\.\d{1,3}"
    r")\b"
)

# The prefix needs 3+ characters. Two-letter prefixes are overwhelmingly
# arithmetic in disguise (`ci-1`, `si-1`, `n-1` inside an embedded awk or JS
# expression), and a gate that trips on `substr(s, 1, ci-1)` gets switched off
# long before it catches a hostname. The cost is a hypothetical two-letter host
# name; the private-address and MAC checks still cover that machine.
HOST_SHAPED = re.compile(r"\b[a-z][a-z0-9]{2,}-[0-9]+\b")

MAC = re.compile(r"\b(?:[0-9a-fA-F]{2}[:-]){5}[0-9a-fA-F]{2}\b")

# An Ansible role path from the private provisioning repo. Deliberately narrow:
# it matches the `roles/<name>/` directory shape and nothing else, which has no
# legitimate occurrence in this tree (see the docstring for why the broader
# snake_case role-name check was rejected). `\b` keeps `user_roles/` and
# `member_roles/` out — there is no word boundary after an underscore.
ANSIBLE_ROLE_PATH = re.compile(r"\broles/[A-Za-z0-9_.-]+/")

# Documentation MACs. Anything else is presumed to be a real NIC.
ALLOWED_MACS = {"aa:bb:cc:dd:ee:ff", "aa-bb-cc-dd-ee-ff", "00:00:00:00:00:00", "ff:ff:ff:ff:ff:ff"}

# `<word>-<digits>` tokens in this tree that are demonstrably not machine names.
# Keep sorted; keep each one obviously not a host.
ALLOWED_TOKENS = {
    "app9003-2970",  # a systemd scope name, `app-steam-app<appid>-<pid>.scope`
    "arch1-1",       # a pacman package version ("6.9.3-arch1-1")
    "arch2-2",       # a pacman package version ("6.6.30.arch2-2")
    "base-16",
    "dev-167",       # a Markdown heading anchor (#dev-control-surface-dev-167)
    "gamescope-0",   # WAYLAND_DISPLAY value gamescope hands its children
    "gamescope-3",   # the SteamOS package version "gamescope-3.16.26-2"
    "incident-1",    # docs/KIOSK_WINDOW_MODEL.md's label for a recorded incident
    "iso-8601",
    "libdbus-1",     # a Debian package name ("libdbus-1-dev")
    "measurements-2026",  # a research-report filename with an ISO date in it
    "mode-0644",     # file-mode prose in a doc comment
    "pre-2026",      # date prose ("the pre-2026-08-07 value")
    "rfc-1918",
    "rfc-5737",
    "scale-2",       # a display scale factor
    "sha-1",
    "sha-256",
    "sha-512",
    "steamos-39",    # a research-report filename ("steamos-39-gamescope-shell.md")
    "usb-0000",      # sysfs/udev device paths
    "usb-1",
    "utf-8",
    "utf-16",
    "wayland-0",     # WAYLAND_DISPLAY values
    "wayland-1",
    "x86-64",
    "zone-2",        # the AV receiver's second zone, a CEC/telnet concept
}

# Generic words that read as a role rather than a machine, when followed by an
# index. These are how the source is *supposed* to talk about nodes.
ALLOWED_PREFIXES = (
    "player-",     # per-player gamepad slots
    "index-",
    "idx-",
    "value-",
    "phase-",      # docs/PANEL_IA.md phase numbers
    "release-",
    "node-",       # the placeholder vocabulary this repo standardized on
)


def is_allowed_token(tok: str) -> bool:
    return tok in ALLOWED_TOKENS or tok.startswith(ALLOWED_PREFIXES)


# --- `#[cfg(test)]` masking -------------------------------------------------

CFG_ATTR = re.compile(r"#\[cfg\((?P<pred>[^\n]*?)\)\]")
TEST_PRED = re.compile(r"(?<![\w-])test(?![\w-])")


def blank_literals(src: str) -> str:
    """Return `src` with comments and literal *bodies* replaced by spaces.

    Same length and same newline positions as the input, so an offset in the
    result addresses the same character in the original. Brace counting has to
    run over this rather than the raw text: a `{` inside `r#"{"node_id":…"#` or
    a `'` opening a lifetime rather than a char literal both corrupt a naive
    count, and a corrupted count ends the `#[cfg(test)]` mask early — which
    turns fixtures back into false positives.
    """
    out = list(src)
    n = len(src)
    i = 0

    def blank(a: int, b: int) -> None:
        for k in range(a, min(b, n)):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        # line comment
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
            continue
        # block comment (Rust nests them)
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            blank(i, j)
            i = j
            continue
        # raw string: r"…", r#"…"#, br##"…"##
        m = re.match(r'(?:b?r)(#*)"', src[i : i + 40])
        if m and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            hashes = m.group(1)
            close = '"' + hashes
            j = src.find(close, i + m.end())
            j = n if j < 0 else j + len(close)
            blank(i, j)
            i = j
            continue
        # normal / byte string
        if c == '"':
            j = i + 1
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1
            blank(i, j)
            i = j
            continue
        # char literal vs lifetime: `'x'`, `'\n'`, `'\u{7f}'` close; `'a` does not
        if c == "'":
            j = i + 1
            if j < n and src[j] == "\\":
                j += 2
                while j < n and src[j] != "'":
                    j += 1
                j += 1
                blank(i, j)
                i = j
                continue
            if j + 1 < n and src[j + 1] == "'":
                blank(i, j + 2)
                i = j + 2
                continue
            i += 1  # a lifetime — nothing to blank
            continue
        i += 1
    return "".join(out)


def test_only_modules(root: Path, rs_files: list[Path]) -> set[Path]:
    """Files reached only through a `#[cfg(test)] mod name;` declaration."""
    decl = re.compile(
        r"#\[cfg\((?P<pred>[^\n]*?)\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(?P<name>\w+)\s*;"
    )
    marked: set[Path] = set()
    for path in rs_files:
        code = blank_literals(path.read_text(encoding="utf-8", errors="replace"))
        for m in decl.finditer(code):
            if not TEST_PRED.search(m.group("pred")):
                continue
            name = m.group("name")
            base = path.parent
            for cand in (base / f"{name}.rs", base / name / "mod.rs"):
                if cand.exists():
                    marked.add(cand.resolve())
            subdir = base / name
            if subdir.is_dir():
                marked.update(p.resolve() for p in subdir.rglob("*.rs"))
    return marked


def non_test_lines(text: str) -> list[tuple[int, str]]:
    """`(1-based line number, text)` for every line outside a `#[cfg(test)]` item."""
    code = blank_literals(text)
    n = len(code)
    masked = bytearray(n)  # 1 ⇒ this character belongs to a cfg(test) item

    for m in CFG_ATTR.finditer(code):
        if not TEST_PRED.search(m.group("pred")):
            continue
        i = m.end()
        depth = 0
        while i < n:
            ch = code[i]
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    i += 1
                    break
            elif ch == ";" and depth == 0:
                i += 1
                break
            i += 1
        for k in range(m.start(), i):
            masked[k] = 1

    keep: list[tuple[int, str]] = []
    offset = 0
    for lineno, line in enumerate(text.splitlines(), start=1):
        if not any(masked[offset : offset + len(line)]):
            keep.append((lineno, line))
        offset += len(line) + 1
    return keep


# --- driver -----------------------------------------------------------------


def tracked_files(root: Path) -> list[Path]:
    out = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [root / p for p in out.split("\0") if p]


def is_skipped_path(rel: str, path: Path) -> bool:
    return (
        bool(TEST_DIRS.intersection(rel.split("/")[:-1]))
        or rel.startswith(VENDORED)
        or path.name.startswith(TEST_FILE_PREFIXES)
    )


def violations_in(rel: str, numbered: list[tuple[int, str]]) -> list[tuple[int, str, str]]:
    check_shape = in_host_shape_scope(rel)
    found = []
    for lineno, line in numbered:
        for ip in PRIVATE_IPV4.findall(line):
            found.append((lineno, f"private IPv4 {ip}", line.strip()))
        for mac in MAC.findall(line):
            if mac.lower() not in ALLOWED_MACS:
                found.append((lineno, f"MAC address {mac}", line.strip()))
        for role in ANSIBLE_ROLE_PATH.findall(line):
            found.append((lineno, f"Ansible role path {role!r}", line.strip()))
        if not check_shape:
            continue
        for tok in HOST_SHAPED.findall(line):
            if not is_allowed_token(tok):
                found.append((lineno, f"host-shaped identifier {tok!r}", line.strip()))
    return found


def main() -> int:
    verbose = "--verbose" in sys.argv
    root = Path(__file__).resolve().parent.parent

    files = [p for p in tracked_files(root) if p.suffix in SOURCE_SUFFIXES and p.is_file()]
    rs_files = [p for p in files if p.suffix == ".rs"]
    test_modules = test_only_modules(root, rs_files)

    scanned = 0
    failures: list[str] = []
    for path in sorted(files):
        rel = path.relative_to(root).as_posix()
        if rel == SELF or is_skipped_path(rel, path) or path.resolve() in test_modules:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        numbered = non_test_lines(text) if path.suffix == ".rs" else list(
            enumerate(text.splitlines(), start=1)
        )
        scanned += 1
        for lineno, why, snippet in violations_in(rel, numbered):
            failures.append(f"{rel}:{lineno}: {why}\n    {snippet}")

    if verbose:
        print(f"scanned {scanned} non-test source files ({len(test_modules)} test-only modules skipped)")

    if failures:
        print("Site identity found in non-test source (docs/PRD.md §5):\n", file=sys.stderr)
        for f in failures:
            print(f, file=sys.stderr)
        print(
            f"\n{len(failures)} violation(s). Use RFC 5737 addresses (192.0.2.x) and "
            "placeholder ids (node-a, <sidecar-host>) in examples; real addresses and "
            "host names belong in config.toml, never in source. Describe what a "
            "private Ansible role DOES rather than naming its roles/<name>/ path.",
            file=sys.stderr,
        )
        return 1

    print(f"No site identity in {scanned} non-test source files.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
