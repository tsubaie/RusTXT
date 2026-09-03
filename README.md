# RustPad

RustPad is a fast, recoverable text editor in the spirit of modern Windows Notepad, built with GTK 4, libadwaita and GtkSourceView in Rust. It targets Linux and macOS.

The goal is to keep everyday text editing immediate and uncluttered while building a dependable foundation for session recovery, Markdown, large files, and optional writing assistance.

## Product principles

- Open quickly and stay out of the way.
- Never lose unsaved work.
- Make tabs and session restoration predictable.
- Keep saved files as ordinary files that remain usable everywhere.
- Use native operating-system behavior where it matters.
- Keep AI and cloud functionality optional.

## Stack

| Layer | Choice |
|---|---|
| Toolkit | GTK 4 + libadwaita through `gtk4-rs` and `libadwaita-rs` |
| Editor widget | GtkSourceView 5 (`sourceview5`) |
| Core | `rustpad-core`: documents, recovery storage, configuration, themes. No GTK dependency, unit-tested alone |
| Recovery and session state | SQLite through `rusqlite`, WAL mode |
| Configuration | TOML in `~/.config/rustpad`, watched with `notify` for live reload |

The filesystem remains authoritative for saved documents. SQLite stores open tabs, unsaved snapshots, cursor and scroll positions, recently closed tabs, and window size.

## What works today

- Tabs (libadwaita tab bar) with per-tab undo history, a `+` button, a right-click tab menu, and Ctrl+Tab switching.
- Notepad-style menu bar: **File**, **Edit**, **View**, plus a settings gear. Native popover menus with mnemonics (`Alt+F`, `Alt+E`, `Alt+V`) and shortcut hints.
- Find and replace in a small floating tool window over the text (`Ctrl+F`, `Ctrl+H`): draggable, with match count, match case, whole word and regular expressions, powered by GtkSourceView's search context. Go to line with `Ctrl+G`.
- Cut, copy, paste, delete, select all, insert time and date (`F5`), and printing through the native print dialog (`Ctrl+P`).
- Zoom (`Ctrl` + wheel, `Ctrl+Plus`, `Ctrl+Minus`, `Ctrl+0`), word wrap and status bar toggles.
- Atomic saves that preserve file permissions and write through symlinks. Line endings are detected on open and preserved on save; the status bar shows `Windows (CRLF)` or `Unix (LF)`.
- Crash-safe recovery: content snapshots are written 400 ms after edits, only for the document that changed. Clean file-backed tabs are re-read from disk on restart so the file stays authoritative.
- "Close for now" keeps a tab's recovery snapshot; **File ▸ Recently closed** and `Ctrl+Shift+T` bring it back. "Discard changes and close" deletes the snapshot after confirmation.
- Opening files from the command line: `rustpad notes.txt other.md`. A second launch hands its files to the running instance.

## Configuration

All settings live in `~/.config/rustpad/config.toml` (`$XDG_CONFIG_HOME/rustpad/config.toml`, the same path on macOS). RustPad writes a commented default on first launch, and every change made in Settings or the View menu is saved back to that file. Edits made by hand apply immediately while RustPad is running.

```toml
[appearance]
theme = "auto"   # "auto", "system", "light", "dark", "omarchy", or a custom theme name
zoom = 100       # 10-500

[editor]
word_wrap = true

[window]
status_bar = true
title_bar = "auto"  # "auto", "show", "hide"
```

### Themes

- `auto` follows the active [Omarchy](https://omarchy.org) theme when one is installed, and the system light/dark preference otherwise.
- `system`, `light`, and `dark` are the stock libadwaita looks with GtkSourceView's Adwaita schemes.
- `omarchy` reads the active theme's `colors.toml` from `~/.local/state/omarchy/current/theme/` and re-applies whenever `omarchy theme set` runs. A theme may also ship its own `rustpad.toml` in that directory to override the derived mapping.
- Any other name loads `~/.config/rustpad/themes/<name>.toml`:

```toml
mode = "dark"            # "dark" or "light"
background = "#1e1e2e"   # editor background
foreground = "#cdd6f4"
accent = "#89b4fa"       # optional; the rest are optional too
muted = "#6c7086"
selection = "#45475a"
border = "#313244"
chrome = "#161622"       # tab strip and window background
menu = "#313244"         # menus, popovers, cards
```

Palettes are applied as libadwaita CSS variables and as a generated GtkSourceView style scheme, so the whole window follows the colors.

### Window title bar

libadwaita draws a client-side header bar. Tiling compositors such as Hyprland, Sway, river and niri never draw a title bar of their own, so `title_bar = "auto"` hides the header bar there and keeps it on GNOME, KDE and macOS. The tab strip doubles as a drag handle when the bar is hidden.

## Session behavior

- **Close for now:** hide the tab while retaining its recovery state; reopen it from **File ▸ Recently closed**.
- **Discard changes:** explicitly and permanently remove the unsaved recovery snapshot.
- **Save:** atomically write the document to disk and update its recovery state.
- **Exit:** preserve all open tabs without forcing save prompts.

## Building

Linux (Arch / Omarchy):

```bash
omarchy pkg add gtk4 libadwaita gtksourceview5   # or: sudo pacman -S gtk4 libadwaita gtksourceview5
cargo run -p rustpad
```

Debian/Ubuntu need `libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev`; Fedora needs `gtk4-devel libadwaita-devel gtksourceview5-devel`.

macOS:

```bash
brew install gtk4 libadwaita gtksourceview5
cargo run -p rustpad
```

Install for the current user:

```bash
cargo install --path crates/rustpad-gtk
install -Dm644 data/com.tsubaie.rustpad.desktop ~/.local/share/applications/com.tsubaie.rustpad.desktop
install -Dm644 data/icons/com.tsubaie.rustpad.svg ~/.local/share/icons/hicolor/scalable/apps/com.tsubaie.rustpad.svg
```

Validation:

```bash
cargo test -p rustpad-core
cargo build --release
```

## Layout

```
crates/rustpad-core   toolkit-free core: storage, files, config, desktop, watch
crates/rustpad-gtk    the application: window, document, find bar, menus, settings, theme, printing
data/                 desktop entry and icon
```

The GTK crate never reaches into SQLite or TOML directly; it calls the core and stays a thin presentation layer.

## Roadmap

- **V2:** encoding and line-ending controls, external-file change detection, recent files, font settings.
- **V3:** Markdown formatting and preview, spellcheck, export.
- **V4:** large-file mode, command palette, advanced navigation, a carefully permissioned extension model.
- **V5:** optional, provider-neutral writing tools backed by local models or user-configured cloud providers.
