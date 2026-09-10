# Iced editor compatibility patches

These are the unmodified published Iced 0.14.0 `iced_graphics` and 0.14.2
`iced_widget` crates, except for the changes listed below. Their MIT licenses
are included. Workspace `[patch.crates-io]` entries pin the copies reproducibly.

`iced_graphics/src/text/editor.rs`:

- Set finite initial buffer bounds before loading text. Otherwise COSMIC shapes
  every line of a file before the first window layout, using substantial memory.
- Use COSMIC's glyph-aware caret and selection geometry for Arabic/RTL text.
- Clear an existing selection when `move_to` requests a caret.
- Expose logical-line/pixel scroll position and restoration after layout.

`iced_widget/src/text_editor.rs`:

- Expose the default renderer's scroll position through `Content` so RusTXT can
  preserve viewports across tab switches and restarts.

Regression tests live in `crates/rustxt-iced/src/document.rs`. Re-evaluate these
patches when updating Iced; do not drop them without running those tests and the
real UI checks. No global Cargo registry files are modified.
