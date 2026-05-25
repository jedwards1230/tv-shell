# Game Shell Color Palette

## Base Accent Colors (shared across themes)

| Name    | Hex     | RGB            | Usage                              |
|---------|---------|----------------|------------------------------------|
| Snow    | #f4f5f7 | (244,245,247)  | Light mode background, text-on-dark|
| Crimson | #c72138 | (199,33,56)    | Focus borders, primary accent      |
| Ember   | #e06236 | (224,98,54)    | Card accents, warnings, restart btn|
| Gold    | #d7a64b | (215,166,75)   | Decorative only, sleep button      |
| Navy    | #304c7a | (48,76,122)    | Volume bar fill, light mode bar    |

## Dark Mode (OLED-optimized, default)

| Token          | Hex     | Usage                              |
|----------------|---------|------------------------------------|
| background     | #111215 | Main background (near-black)       |
| surface        | #33363f | Cards, panels, sidebar             |
| surfaceHover   | #424650 | Hovered/focused surface elements   |
| surfaceBorder  | #4d525c | Borders, dividers                  |
| textPrimary    | #e6e4e0 | Headings, labels, body text        |
| textSecondary  | #c2bfba | Subtitles, metadata                |
| textMuted      | #928e88 | Hints, disabled text               |
| cardBackground | #2e3139 | Card fill                          |
| barBackground  | #111215 | Status bar                         |
| sidebarActive  | #424650 | Active sidebar section highlight   |
| sidebarText    | #e6e4e0 | Sidebar text                       |

## Light Mode

| Token          | Hex     | Usage                              |
|----------------|---------|------------------------------------|
| background     | #f4f5f7 | Main background (snow)             |
| surface        | #ffffff | Cards, panels                      |
| surfaceHover   | #ecedf0 | Hovered/focused surface elements   |
| surfaceBorder  | #dcdee3 | Borders, dividers                  |
| textPrimary    | #1a2540 | Headings, labels (navy-tinted)     |
| textSecondary  | #4a5568 | Subtitles, metadata                |
| textMuted      | #8892a4 | Hints, disabled text               |
| cardBackground | #ffffff | Card fill                          |
| barBackground  | #304c7a | Status bar (navy)                  |
| sidebarActive  | #304c7a | Active sidebar section (navy)      |
| sidebarText    | #e6e4e0 | Sidebar text (always light)        |

## Shared Tokens (theme-independent)

| Token          | Hex       | Usage                              |
|----------------|-----------|--------------------------------------|
| textOnDark     | #f4f5f7   | Text on dark backgrounds (snow)    |
| textOnDarkMuted| #d8d5d0   | Muted text on dark backgrounds     |
| online         | #2d8a4e   | Online/connected indicator         |
| offline        | (crimson) | Offline/disconnected indicator     |
| warning        | (ember)   | Warning states                     |
| focusBorder    | (crimson) | Focus ring on interactive elements |
| focusGlow      | #c7213833 | Focus glow (crimson at 20% alpha)  |

## Theme Modes

- **Dark** (default): OLED-optimized near-black background
- **Light**: Snow-white background with navy-tinted grays
- **Auto**: Time-based switching (dark 8 PM - 7 AM, light otherwise)

Persisted to `~/.config/game-shell/settings.json`.

## Rules

- NEVER use gold for text -- only as decorative accents (sleep button)
- Use crimson for focus/active states and borders
- Use ember for secondary interactive elements and warnings
- Text on dark backgrounds (navy bar, overlays) is always snow/white
- All overlay backdrops use Qt.rgba(0, 0, 0, 0.7-0.85)
