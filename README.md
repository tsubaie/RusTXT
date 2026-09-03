# RustPad

RustPad is a fast, recoverable, cross-platform text editor inspired by the simplicity of modern Windows Notepad.

The goal is to keep everyday text editing immediate and uncluttered while building a dependable foundation for session recovery, Markdown, large files, and optional writing assistance. RustPad targets Windows, macOS, and Linux from one codebase.

## Product principles

- Open quickly and stay out of the way.
- Never lose unsaved work.
- Make tabs and session restoration predictable.
- Keep saved files as ordinary files that remain usable everywhere.
- Use native operating-system behavior where it matters.
- Keep AI and cloud functionality optional.

## Stack

- **Desktop shell:** Tauri 2
- **Editor:** CodeMirror 6
- **Application core:** Rust, split into Tauri-free modules (`storage`, `files`, `config`, `desktop`, `watch`) behind a thin `commands` boundary
- **Recovery and session state:** SQLite through `rusqlite`, using WAL mode
- **Configuration:** TOML in the XDG config directory, watched with `notify` for live reload
- **Packaging and updates:** Tauri bundles and updater

The filesystem remains authoritative for saved documents. SQLite stores open tabs, unsaved snapshots, cursor and scroll positions, recently closed tabs, and crash-recovery metadata.

## What works today (V1)

- Tabs with per-tab undo history, a `+` button, middle-click close, and a right-click tab menu.
- Windows 11 Notepad style menu bar: **File**, **Edit**, **View**, and a settings gear. Menus support hover switching, submenus, check marks, shortcut hints, keyboard navigation, and `Alt+F` / `Alt+E` / `Alt+V`.
- Find and replace in a small floating tool window over the text (`Ctrl+F`, `Ctrl+H`), with match count, match case, whole word, and regular expressions. Go to line with `Ctrl+G`.
- Cut, copy, paste, and delete from the menu through the system clipboard; select all; insert time and date with `F5`; print with `Ctrl+P`.
- Zoom (`Ctrl` + wheel, `Ctrl+Plus`, `Ctrl+Minus`, `Ctrl+0`), word wrap, and status bar toggles.
- Atomic saves that preserve file permissions and write through symlinks. Line endings are detected on open and preserved on save; the status bar shows `Windows (CRLF)` or `Unix (LF)`.
- Crash-safe recovery: content snapshots are written 400 ms after edits, only for the document that changed. Clean file-backed tabs are re-read from disk on restart so the file stays authoritative.
- "Close for now" keeps a tab's recovery snapshot; **File ▸ Recently closed** and `Ctrl+Shift+T` bring it back. "Discard changes and close" deletes the snapshot after confirmation.

## Configuration

All settings live in `~/.config/rustpad/config.toml` (`$XDG_CONFIG_HOME/rustpad/config.toml`). RustPad writes a commented default on first launch, and every change made in the Settings page or the View menu is saved back to that file. Edits made to the file by hand apply immediately while RustPad is running.

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
- `system`, `light`, and `dark` are the built-in Notepad-like looks.
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
chrome = "#161622"       # tab strip
menu = "#313244"         # menus and popups
```

### Window title bar

Tauri windows ask for native decorations, and on Wayland GTK draws its own header bar when the compositor does not. Tiling compositors such as Hyprland, Sway, river, and niri never draw one, so `title_bar = "auto"` hides the GTK bar there and keeps native decorations on GNOME, KDE, Windows, and macOS. The tab strip doubles as a drag handle when the bar is hidden.

## Session behavior

RustPad deliberately distinguishes among these actions:

- **Close for now:** hide the tab while retaining its recovery state; reopen it from **File ▸ Recently closed**.
- **Discard changes:** explicitly and permanently remove the unsaved recovery snapshot.
- **Save:** atomically write the document to disk and update its recovery state.
- **Exit:** preserve all open tabs without forcing save prompts.

## Roadmap

### V2 — Everyday editor tools

Encoding and line-ending controls, external-file change detection, recent files, and font settings.

### V3 — Rich text workflows

Markdown formatting and preview, spellcheck, and export-oriented improvements.

### V4 — Power-user foundation

Large-file mode, command palette, advanced navigation, and a carefully permissioned extension model.

### V5 — Optional Writing Tools

Provider-neutral rewrite, summarize, and compose actions backed by local models or user-configured cloud providers.

## Development

Prerequisites are Node.js, npm, the stable Rust toolchain, and the platform dependencies required by Tauri 2.

```bash
npm install
npm run tauri dev
```

Validation commands:

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
npm run tauri build -- --debug --no-bundle
```

The Rust core never depends on Tauri types outside `commands.rs` and `lib.rs`, so storage, file handling, configuration, theme resolution, and desktop detection are unit-tested on their own.
