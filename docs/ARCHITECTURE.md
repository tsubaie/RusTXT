# Architecture

RustPad is a Rust workspace with two crates.

```
crates/rustpad-core   documents, recovery storage, config, themes. No GTK. Unit tested on its own.
crates/rustpad-gtk    the application: window, tabs, find bar, menus, settings, printing.
data/                 desktop entry and icon.
packaging/            Arch PKGBUILDs and the tarball install script used by the release workflow.
```

- **GTK 4 + libadwaita** through `gtk4-rs`, **GtkSourceView 5** for the editor widget.
- **SQLite** (WAL mode) stores open tabs, unsaved snapshots, cursor and scroll positions, recently closed tabs and the window size. Only the tab being edited is written, 400 ms after the last change.
- **TOML** configuration in `~/.config/rustpad/config.toml`, watched with `notify` so hand edits and Omarchy theme changes apply live. Omarchy palettes come from `~/.local/state/omarchy/current/theme/colors.toml` and are applied as libadwaita CSS variables plus a generated GtkSourceView style scheme.
- Saves go through a temporary file and an atomic rename, keep the original permissions, and write through symlinks. Line endings are detected on open and re-applied on save.
- Window decorations are decided at runtime: hidden on tiling compositors (Hyprland, Sway, river, niri), native elsewhere, with a user override.

```bash
cargo test -p rustpad-core      # core unit tests
cargo build --release           # optimized binary with LTO
```

Releases are built by `.github/workflows/release.yml` on a `v*` tag: Arch package and tarball in an Arch container, `.deb` with cargo-deb on Debian, `.rpm` with cargo-generate-rpm on Fedora, then a GitHub Release with checksums.
