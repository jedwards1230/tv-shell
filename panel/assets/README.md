# Vendored panel assets

These files are served by the `tv-shell-panel` binary itself (embedded at compile
time via `include_str!`), so the panel renders with no CDN and no network — it
must work when the rest of the system is broken.

## `htmx.min.js`

- **Library**: [htmx](https://htmx.org/)
- **Version**: 2.0.4
- **Source**: `https://unpkg.com/htmx.org@2.0.4/dist/htmx.min.js`
  (identical byte-for-byte to the jsdelivr copy of the same release)
- **SHA-256**: `e209dda5c8235479f3166defc7750e1dbcd5a5c1808b7792fc2e6733768fb447`
- **License**: BSD 2-Clause (compatible with this repo's GPL-3.0)

Committed verbatim from the official release — do not hand-edit. To update, fetch
the new release, re-verify its hash against the published artifact, and update the
version + hash above in the same commit.

## `style.css`

Hand-written admin stylesheet for the panel (dark, minimal). Not vendored.
