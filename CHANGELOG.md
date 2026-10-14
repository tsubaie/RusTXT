# Changelog

All notable changes to RusTXT are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Release notes that
accompany each GitHub Release live in `docs/releases/<version>.md`.

## [Unreleased]

### Removed

- The GTK frontend (`crates/rustxt-gtk`, the `rustxt` binary of 0.4.x) and its
  documentation. The last revision that contains it is tagged `v0.4.1-gtk`.
- The in-app updater in `rustxt-core`. The About dialog only links to the
  releases page; the app never downloads or replaces its own executable.

### Changed

- Undo and redo step through typed words instead of single characters: a run of
  characters typed without a space or line break is one undo step.

- The `ci.yml` and `iced.yml` workflows are merged into one CI workflow, and
  `make check` runs exactly the gates CI runs.
- The recovery database (`session.db`) is created with mode `0600` on Unix.
- Lone CR line endings are normalised to LF when a file is loaded; CRLF files
  are still preserved as CRLF.
- Files larger than 64 MiB and non-regular files (devices, FIFOs, directories)
  are refused with an error instead of being loaded.
- *Discard changes and close* also deletes the note's undo history from the
  recovery database, not only its snapshot.

### Added

- `CHANGELOG.md`, `SECURITY.md`, `CONTRIBUTING.md`, `.editorconfig`, a bug
  report issue template, a pull request template, and `docs/configuration.md`.

### Fixed

- Saving no longer replaces a read-only file through the atomic rename; it is
  refused with a message suggesting Save As.
- Zooming with the keyboard or wheel was reverted by the configuration watcher
  echoing the app's own save; in-flight settings now survive that reload.
- Several files failing to open at startup showed only the last error; all of
  them are listed, each named.
- An invalid theme name in the configuration is reported instead of silently
  falling back.
- Storage failures such as a full disk say what they mean for unsaved drafts.
- Long tab titles are truncated on one line instead of wrapping.

- The first visible editor line painted over the menu bar and became bold after
  partial redraws.

## [0.5.0] - 2026-09-10

### Changed

- RusTXT uses Iced on Linux, Windows, and macOS. GTK is no longer required by the
  released editor. Linux, Windows (portable ZIP, C runtime included, unsigned),
  and macOS (Apple Silicon and Intel, ad-hoc signed, not notarized) packages are
  built by the release workflow.
- Restored the familiar tabs, menus, grouped Settings, and compact Find/Replace
  panel in the Iced interface; menus are drawn in the window on every platform.
- Printing opens a local print view in the default browser.

### Added

- Readable, opaque tooltips explaining every Find/Replace button.
- Native startup smoke tests covering Unicode filenames, single-instance
  forwarding, and recovery on all four CI platforms; Linux real-window
  editing/recovery and hover-rendering end-to-end tests; clean-container
  installation tests for the Arch, Debian, and Fedora packages.

### Fixed

- Menu and dialog darkening during hover redraws, including at fractional display
  scaling (vendored `iced_tiny_skia` shadow clipping fix).

### Preserved

- Recovery snapshots, undo history, cursor and scroll positions, safe saves, and
  Arabic/English editing from the 0.4.x releases. The recovery database and
  configuration are shared with 0.4.x; close the old version before starting
  0.5.0.

[Unreleased]: https://github.com/tsubaie/RusTXT/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/tsubaie/RusTXT/compare/v0.4.1...v0.5.0
