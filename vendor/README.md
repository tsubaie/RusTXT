# Iced editor compatibility patches

These are the unmodified published Iced 0.14.0 `iced_graphics` 0.14.2
`iced_widget`, and 0.14.1 `iced_tiny_skia` crates, except for the changes listed below. Their MIT licenses
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

`iced_tiny_skia/src/engine.rs`:

- Clip shadows to the current damaged region/layer. Upstream paints them outside
  the cleared region on partial redraws, repeatedly darkening menus and dialogs.
- Include shadow extents in visibility checks and clamp shadow rasterization to
  damaged, on-screen pixels. This also avoids rebuilding a full dialog-sized
  shadow for every hovered control.
- The renderer regression test compares repeated partial redraws with a full
  render at 100%, 125%, 150%, and 200% scale. Run it with:
  `cargo test -p iced_tiny_skia --lib --locked`.
