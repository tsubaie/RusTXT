# Configuration

RusTXT keeps one configuration file. It is written with comments on first
launch, kept in sync with the Settings dialog, and re-read immediately when
edited by hand (the configuration directory, the themes directory, and the
Omarchy theme directory are watched while the app runs).

## File locations

| Platform | Configuration | Custom themes | Recovery database | Cache (print view) |
|---|---|---|---|---|
| Linux | `$XDG_CONFIG_HOME/rustxt/config.toml` (default `~/.config/rustxt/config.toml`) | `~/.config/rustxt/themes/` | `$XDG_DATA_HOME/rustxt/session.db` (default `~/.local/share/rustxt/session.db`) | `$XDG_CACHE_HOME/rustxt/` (default `~/.cache/rustxt/`) |
| macOS | `~/.config/rustxt/config.toml` | `~/.config/rustxt/themes/` | `~/.local/share/rustxt/session.db` | `~/.cache/rustxt/` |
| Windows | `%APPDATA%\rustxt\config.toml` | `%APPDATA%\rustxt\themes\` | `%LOCALAPPDATA%\rustxt\session.db` | `%LOCALAPPDATA%\rustxt\cache\` |

macOS deliberately uses the same dotfile layout as Linux. `XDG_*` variables are
honoured on every platform when set to an absolute path; `HOME` (or
`USERPROFILE`) is the fallback base.

`RUSTXT_DATA_DIR=<dir>` overrides the recovery database directory only, on all
platforms. It is the way to start a separate session for testing:

```sh
RUSTXT_DATA_DIR=/tmp/rustxt-review rustxt
```

If the editor's previous `rustpad` configuration or data directory exists and
the `rustxt` one does not, it is renamed on first start so nothing is lost.

## Keys

All keys are optional. Missing sections and keys take the defaults below;
unknown keys are ignored. If the file cannot be parsed the app runs with the
defaults and reports the parse error instead of silently falling back.

| Key | Type | Default | Allowed values | Notes |
|---|---|---|---|---|
| `appearance.theme` | string | `"auto"` | `"auto"`, `"system"`, `"light"`, `"dark"`, `"omarchy"`, or the name of a file in `themes/` without `.toml` | `"auto"` uses the Omarchy theme when `colors.toml` is present and the system light/dark setting otherwise. Blank is treated as `"auto"`. A custom name that cannot be loaded falls back to the system mode with a note in Settings. |
| `appearance.zoom` | integer | `100` | `10`–`500` | Editor zoom in percent. Values outside the range are clamped on load. |
| `editor.word_wrap` | boolean | `true` | `true`, `false` | |
| `editor.font` | string | `""` | `"<family> <size>"` or `""` | See [Font](#font). |
| `window.status_bar` | boolean | `true` | `true`, `false` | |
| `window.title_bar` | string | `"auto"` | `"auto"`, `"show"`, `"hide"` | `"auto"` hides the native title bar on tiling compositors such as Hyprland and keeps it elsewhere; the other two force it. Lowercase only. |

A complete file with the defaults:

```toml
[appearance]
theme = "auto"
zoom = 100

[editor]
word_wrap = true
font = ""

[window]
status_bar = true
title_bar = "auto"
```

### Font

`editor.font` is a single font family name followed by a size, separated by a
space, for example `"JetBrains Mono 12"` or `"Fira Code 11"`. The app splits the
string at the last space:

- The part before it is the family name (spaces inside the family name are fine).
- The part after it must parse as a number and is the size in points; it is
  converted to pixels (×4/3) and clamped to 6–96 px.
- If the trailing word is not a number, the whole string is used as the family
  name at the default size.
- An empty string uses the system monospace font (`fc-match monospace` on
  Linux, the toolkit's monospace default elsewhere) at the default size of
  15 px (about 11 pt).

Only one family is honoured. If a comma-separated list such as
`"Fira Code, Amiri 12"` is given, everything after the first comma is ignored.
Scripts the chosen family does not cover (Arabic, emoji, …) are rendered with
whatever fallback fonts the text engine finds on the system; there is no
setting to pick the fallback face.

## Themes

`"system"`, `"light"` and `"dark"` are built in. `"omarchy"` reads the active
[Omarchy](https://omarchy.org) theme: `rustxt.toml` inside
`~/.local/state/omarchy/current/theme/` if the theme ships one, otherwise a
palette derived from that directory's `colors.toml`. The app re-themes as soon
as `omarchy theme set` switches the directory.

A custom theme is a TOML file in the themes directory. Set `theme` to its file
name without the extension:

```toml
# ~/.config/rustxt/themes/solarized.toml
mode = "dark"            # required: "dark" or "light"
background = "#002b36"   # required
foreground = "#839496"   # required
accent = "#b58900"       # optional, as are the rest
muted = "#586e75"
selection = "#073642"
border = "#073642"
chrome = "#00212b"       # tab strip and window
menu = "#073642"         # menus and popups
```

`mode`, `background` and `foreground` are required; a theme missing any of
them is rejected and the app falls back to the system mode. Omitted optional
colours are derived from the required ones.
