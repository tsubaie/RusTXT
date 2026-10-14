# Golden screenshots

Reference captures for `tools/iced-golden-e2e.py` (`make goldens`): five UI
states, each at 100 % and 125 % display scale, named `<state>@<scale>.png`.

| State | What is on screen |
|---|---|
| `editor` | A fixed English/Arabic document with the cursor on line 3, column 5 |
| `file-menu` | The File menu open over that document |
| `settings` | The Settings panel |
| `find-bar` | The find bar with the query `the` selected as match "1 of 9" |
| `error-dialog` | The startup error for a missing file |

## Where they come from

The goldens are produced on the CI Ubuntu image (`ubuntu-22.04`, with
`fonts-dejavu-core` and `fonts-noto-core`), not on a developer machine: glyph
shapes, hinting and the Arabic fallback face differ between font packages and
versions, and a golden made elsewhere fails on CI by far more than the
tolerance. Goldens generated locally are only useful to exercise the
comparison path.

## Refreshing them

1. Run the CI workflow manually (Actions → CI → *Run workflow*) with
   `update_goldens` ticked. The Linux job then runs `make goldens-update`
   instead of the comparison and uploads `tools/goldens` as the `goldens`
   artifact.
2. Download the artifact, unpack it over `tools/goldens/`, look at every
   changed image (the diff against the previous golden is the review), and
   commit it with the change that caused it.

Locally, `make goldens-update` rewrites the files from your own machine, and
`make goldens` compares against them. One run at a time: the sandbox lives at
a fixed path (`/tmp/rustxt-golden`), because the Settings panel prints the
configuration paths, so concurrent runs would share one app instance.

The comparison is not part of `make e2e` or `make check`. CI runs it after
`make e2e` only when `tools/goldens/*.png` exist.

## Tolerance

Two correct renders of the same frame are not byte-identical: Xvfb's 24-bit
visual, wgpu's sRGB conversion and tiny-skia's blending each round
differently (a unit or two per channel), the caret blinks every 500 ms, and a
glyph may land on a slightly different sub-pixel offset. The comparison
therefore accepts

- a per-channel difference of up to 8 units (of 255) on any pixel, and
- up to 0.5 % of pixels differing by more than that (about 3,500 px of a
  1000x700 window).

A panel that moved, a line of text that is missing or wrapped, a colour that
changed or a popup that did not open exceeds the budget by an order of
magnitude; the caret and anti-aliasing noise stay two orders below it. On a
mismatch the script writes `<state>@<scale>.actual.png` and a
`<state>@<scale>.diff.png` (differences highlighted in red) into its output
directory, `target/iced-golden`, which CI uploads as `iced-ui-failure`.

## What is pinned

`tools/iced-golden-e2e.py` fixes every known source of variance: window size
1000x700 logical, `theme = "dark"`, `editor.font = "DejaVu Sans Mono 11"`,
the title bar hidden, a sandbox at a fixed path, the compositor-detection
environment variables removed, the pointer parked off the window so nothing
is hovered, and each capture taken only once two consecutive captures agree.
The status bar shows only the cursor position, character count, zoom, line
ending and encoding, so it is left on.
