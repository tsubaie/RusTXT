<p align="center"><img src="data/icons/com.tsubaie.rustxt.png" alt="RusTXT" width="180"></p>

# RusTXT

A small, recoverable plain text editor built with Rust and Iced for Linux,
Windows, and macOS.

This branch migrates the interface from GTK to Iced. `cargo run` and `make run`
start the Iced frontend. The GTK frontend remains available for comparison with
`cargo run -p rustxt`; the published 0.4.x packages still contain GTK.

![Iced editor with Arabic and English](docs/screenshots/iced-editor.png)

## Features

- Tabs, keyboard shortcuts, file drag and drop, and single-instance file forwarding.
- SQLite recovery snapshots every 250 ms while editing, including unsaved notes.
- Undo/redo history and cursor/scroll positions restored across restarts.
- Close a tab and reopen it with Ctrl+Shift+T. Permanent discard requires confirmation.
- Find and replace with match counting, case sensitivity, whole words, and regex captures.
- Atomic file saves, external-change protection, symlink handling, and CRLF preservation.
- Arabic/English editing, Unicode font fallback, word wrap, zoom, and a status bar.
- System, light, dark, Omarchy, and custom themes; live configuration reloads.
- Native file dialogs. Printing opens a local print view in the default browser.

The interface uses in-window menus. On macOS, command shortcuts use Cmd; tab
cycling uses Ctrl+Tab. The About dialog links to the latest release rather than
replacing the running executable with an older GTK release.

## Build and run

Use a current stable Rust toolchain:

```sh
cargo run --release -p rustxt-iced
cargo run --release -p rustxt-iced -- notes.txt
```

Linux needs the usual X11/Wayland and keyboard libraries. On Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libfontconfig1-dev libx11-dev libxrandr-dev libxi-dev
```

File dialogs on Linux use the desktop's XDG portal. No GTK, libadwaita,
GtkSourceView, GPU, or browser runtime is required for the editor itself.
The renderer is Iced's CPU-based `tiny-skia` backend.

Windows uses the MSVC Rust toolchain and Visual Studio C++ build tools. macOS
uses the Rust toolchain and Xcode command-line tools. The `Iced cross-platform`
workflow tests/builds all three platforms and produces a Linux tarball, a Windows
portable ZIP, and a macOS application bundle. These are development artifacts;
the macOS bundle is ad-hoc signed, not notarized.

## Existing notes and settings

Iced reads the same SQLite recovery database and TOML configuration as GTK.
Close GTK before opening Iced against the same session. To try a separate session:

```sh
RUSTXT_DATA_DIR=/tmp/rustxt-iced-review cargo run --release -p rustxt-iced
```

This changes the recovery location; appearance settings still use the normal
configuration directory. Linux and macOS retain the existing XDG/HOME layout:
`~/.config/rustxt/config.toml` and `~/.local/share/rustxt/session.db`. Windows uses
`%APPDATA%/rustxt` for configuration and `%LOCALAPPDATA%/rustxt` for recovery.

Undo history is stored as compact edits with an approximately 8 MiB per-tab
budget, retaining at least the latest edit. Files remain ordinary UTF-8 text.

## Development

```sh
make check        # formatting, clippy, core/editor tests
make build        # target/release/rustxt-iced
make e2e          # Linux: Xvfb, xdotool, ImageMagick, Python 3
```

```
crates/rustxt-core   recovery, safe file writes, configuration, themes
crates/rustxt-iced   the cross-platform frontend
crates/rustxt-gtk    the previous GTK frontend
vendor/             pinned Iced editor fixes, documented in vendor/README.md
```

See [migration and footprint report](docs/iced-migration.md) for measurements,
validation, and platform limits. The previous frontend's documentation is in
[GTK documentation](docs/gtk.md).

## License

MIT. Vendored Iced components retain their upstream MIT licenses.
