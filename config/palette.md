# Game Shell Color Palette

## Base Colors (5)

| Name    | Hex     | RGB            | Usage                    |
|---------|---------|----------------|--------------------------|
| Snow    | #f4f5f7 | (244,245,247)  | Backgrounds, text-on-dark |
| Crimson | #c72138 | (199,33,56)    | Focus borders, primary accent |
| Ember   | #e06236 | (224,98,54)    | Card accents, warnings   |
| Gold    | #d7a64b | (215,166,75)   | Decorative elements only |
| Navy    | #304c7a | (48,76,122)    | Status bar, sidebar, dark surfaces |

## Derived Semantic Colors

Text hierarchy is navy-tinted grays:
- textPrimary: #1a2540 (near-black)
- textSecondary: #4a5568 (medium)
- textMuted: #8892a4 (light)
- textOnDark: #f4f5f7 (snow)

## Rules

- NEVER use gold for text — only as decorative card accent bars
- Use crimson for focus/active states
- Use ember for secondary interactive elements and warnings
- Text on dark backgrounds (navy bar) is always snow/white
