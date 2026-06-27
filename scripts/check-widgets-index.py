#!/usr/bin/env python3
"""
Consistency check: widgets-index.json must mirror WidgetManifests.qml.

WidgetManifests.qml is the authoring SSOT for per-widget id/version/requires.
widgets-index.json is kept in-sync and serves as the machine-readable catalog
for future runtime loaders and the release-widget.yml gate.

Exits 1 with a clear error if any drift is detected.
Run from the repo root, or any depth — paths are resolved relative to this script.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
QML_PATH = ROOT / "shell/widgets/lib/WidgetManifests.qml"
INDEX_PATH = ROOT / "widgets-index.json"


def extract_widget_blocks(text):
    """Extract top-level { } blocks from inside the manifests: [ ... ] array."""
    m = re.search(r"readonly property var manifests:\s*\[", text)
    if not m:
        sys.exit(
            "ERROR: Cannot find 'readonly property var manifests: [' in WidgetManifests.qml"
        )

    # Walk forward from the opening '[', tracking bracket depth to find the array end.
    arr_start = m.end()
    depth = 1
    i = arr_start
    while i < len(text) and depth > 0:
        if text[i] == "[":
            depth += 1
        elif text[i] == "]":
            depth -= 1
        i += 1
    arr_text = text[arr_start : i - 1]

    # Collect top-level { } widget blocks within the array.
    blocks = []
    depth = 0
    start = None
    for j, ch in enumerate(arr_text):
        if ch == "{":
            if depth == 0:
                start = j
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0 and start is not None:
                blocks.append(arr_text[start : j + 1])
                start = None
    return blocks


def parse_block(block):
    """Parse id, version, requires from a single widget manifest block."""
    id_m = re.search(r'"id":\s*"([^"]+)"', block)
    version_m = re.search(r'"version":\s*"([^"]+)"', block)
    # requires is always a single-line array: "requires": ["a", "b"] or "requires": []
    requires_m = re.search(r'"requires":\s*\[([^\]]*)\]', block)

    if not id_m:
        sys.exit(f"ERROR: Widget block missing 'id': {block[:80].strip()!r}")
    if not version_m:
        sys.exit(f"ERROR: Widget '{id_m.group(1)}' missing 'version'")

    raw_req = requires_m.group(1) if requires_m else ""
    requires = sorted(
        r.strip().strip('"') for r in raw_req.split(",") if r.strip().strip('"')
    )
    return {
        "id": id_m.group(1),
        "version": version_m.group(1),
        "requires": requires,
    }


def main():
    for path in (QML_PATH, INDEX_PATH):
        if not path.exists():
            sys.exit(f"ERROR: {path} not found (run from repo root or any subdirectory)")

    qml_text = QML_PATH.read_text()
    blocks = extract_widget_blocks(qml_text)
    qml_widgets = [parse_block(b) for b in blocks]

    index = json.loads(INDEX_PATH.read_text())
    index_widgets = index.get("widgets", [])

    qml_by_id = {w["id"]: w for w in qml_widgets}
    idx_by_id = {w["id"]: w for w in index_widgets}

    errors = []

    for wid, qml in qml_by_id.items():
        if wid not in idx_by_id:
            errors.append(
                f"  '{wid}': present in WidgetManifests.qml but missing from widgets-index.json"
            )
            continue
        idx = idx_by_id[wid]
        if qml["version"] != idx.get("version"):
            errors.append(
                f"  '{wid}' version mismatch:"
                f" WidgetManifests.qml={qml['version']!r},"
                f" widgets-index.json={idx.get('version')!r}"
            )
        qml_req = qml["requires"]
        idx_req = sorted(idx.get("requires", []))
        if qml_req != idx_req:
            errors.append(
                f"  '{wid}' requires mismatch:"
                f" WidgetManifests.qml={qml_req},"
                f" widgets-index.json={idx_req}"
            )

    for wid in idx_by_id:
        if wid not in qml_by_id:
            errors.append(
                f"  '{wid}': present in widgets-index.json but missing from WidgetManifests.qml"
            )

    if errors:
        print("FAIL: widgets-index.json is out of sync with WidgetManifests.qml:")
        for e in errors:
            print(e)
        print()
        print(
            "Update widgets-index.json to match WidgetManifests.qml"
            " (WidgetManifests.qml is the authoring SSOT)."
        )
        sys.exit(1)

    print(
        f"OK: widgets-index.json matches WidgetManifests.qml"
        f" ({len(qml_widgets)} widgets checked)"
    )


if __name__ == "__main__":
    main()
