# Contributing to RusTXT

Issues and pull requests are welcome. Keep the core toolkit-free and covered by
tests, and keep the interface boring in the best way: if a feature needs a
manual, it probably does not belong in a plain text editor.

## Development setup

- A current **stable** Rust toolchain (`rustup default stable`). The workspace
  uses `Cargo.lock` and every gate runs with `--locked`.
- Linux build dependencies, on Debian/Ubuntu:

  ```sh
  sudo apt install build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libfontconfig1-dev libx11-dev libx11-xcb1 libxcursor1 libxrandr-dev libxi-dev
  ```

  Arch: `libxkbcommon libxkbcommon-x11 wayland libx11 libxcursor libxi fontconfig`.
  Fedora: `libxkbcommon-devel wayland-devel fontconfig-devel libX11-devel libXrandr-devel libXi-devel`.
  Windows needs the MSVC toolchain and Visual Studio C++ build tools; macOS
  needs the Xcode command-line tools.
- Run `make setup` once per clone. It points `core.hooksPath` at `.githooks`,
  so every `git push` runs `make check` first (`git push --no-verify` skips it
  once).

```sh
make check   # cargo fmt --check, clippy -D warnings, unit tests, renderer regression
make build   # target/release/rustxt-iced
make run     # cargo run -p rustxt-iced
make e2e     # Linux only, see below
```

`make check` runs exactly the gates CI runs: formatting, Clippy with warnings
denied for `rustxt-core` and `rustxt-iced`, the unit tests of both crates, and
the vendored `iced_tiny_skia` renderer regression test. If `make check` passes
locally, the lint/test job in CI passes too.

## Test layers

1. **Unit tests** (`cargo test -p rustxt-core -p rustxt-iced`): recovery
   storage, safe file writes, configuration and themes, the document/undo
   model, and the regression tests for the vendored Iced editor patches
   (`crates/rustxt-iced/src/document.rs`). Add a test next to the code you
   change; every bug fix should come with the test that failed before it.
2. **Renderer regression** (`cargo test -p iced_tiny_skia --lib --locked`):
   compares repeated partial redraws with a full render at 100 %, 125 %, 150 %,
   and 200 % scale. Run it whenever `vendor/iced_tiny_skia` changes.
3. **Linux end-to-end tests** (`make e2e`): `tools/iced-e2e.py` drives the real
   release binary under Xvfb with xdotool (typing, undo/redo, kill-and-recover,
   tabs, find/replace, single-instance forwarding, CRLF saves, external-change
   protection, settings, scroll recovery, clean exit) and
   `tools/iced-rendering-e2e.py` checks hover redraws at 125 % scale. They
   need:

   | Distribution | Packages |
   |---|---|
   | Debian/Ubuntu | `xvfb xdotool imagemagick python3` (plus `fonts-noto-core` for the Arabic sample) |
   | Arch | `xorg-server-xvfb xdotool imagemagick python` |
   | Fedora | `xorg-x11-server-Xvfb xdotool ImageMagick python3` |

   Failures leave screenshots and logs in `target/iced-e2e` and
   `target/iced-rendering-e2e`. The Arabic/English screenshot (`editor.png`) is
   captured for manual review; it is not asserted on.
4. **Native startup smoke test** (`python tools/iced-startup-smoke.py
   target/release`): runs in CI on Windows and macOS, and inside clean Arch,
   Debian, and Fedora containers against the installed packages. It checks
   Unicode filenames, forwarding a second launch into the first instance, and
   recovery after killing the process.

CI runs layers 1–2 on Linux, Windows, Apple Silicon macOS, and Intel macOS,
and layers 3–4 where the platform allows. The release workflow repeats all of
them before publishing.

## Vendored Iced patches

`vendor/` holds the published `iced_graphics`, `iced_widget`, and
`iced_tiny_skia` crates with a handful of local fixes (RTL caret geometry,
lazy shaping, scroll restoration, shadow clipping). The workspace pins them
through `[patch.crates-io]`; no global Cargo registry files are touched.
Every patch is listed in [vendor/README.md](vendor/README.md) with the test
that guards it. When you change or update a vendored crate:

- keep that README's list accurate;
- keep the regression tests enabled and passing (`make check` runs them);
- prefer upstreaming: if a fix lands in Iced, drop the local copy and note it in
  the changelog.

## Commits and pull requests

- One logical change per commit, with a message that says what changed and why.
  Reference the issue when there is one.
- Run `make check` before pushing (the hook does it for you).
- Add or update tests for behaviour changes. Changes to loading, saving, or
  the recovery database need unit tests in `rustxt-core` and, where a real
  window matters, a step in `tools/iced-e2e.py`.
- Add a line under **Unreleased** in [CHANGELOG.md](CHANGELOG.md) for anything
  a user or packager would notice.
- Update the README or `docs/` when you change configuration keys, shortcuts,
  dependencies, or file locations; the tables there are checked against the
  code during review.
- The pull request template has the checklist; fill it in rather than deleting
  it.

## Release procedure

Releases are built and published by `.github/workflows/release.yml` from a
tag. To cut `X.Y.Z`:

1. Set `version = "X.Y.Z"` under `[workspace.package]` in the root
   `Cargo.toml` and refresh `Cargo.lock` (`cargo update -w` or a build). The
   workflow refuses a tag whose version differs from the `rustxt-iced` package
   version.
2. Move the **Unreleased** entries in `CHANGELOG.md` under a new
   `## [X.Y.Z] - YYYY-MM-DD` heading and update the comparison links at the
   bottom.
3. Write `docs/releases/X.Y.Z.md`. The workflow uses it as the GitHub Release
   body, so it should be the user-facing summary with the download table.
4. Commit, then tag and push:

   ```sh
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```

   The workflow validates the version, builds the Arch, Debian, and Fedora
   packages, the Linux tarball, the macOS bundles, and the Windows ZIP, runs
   the native smoke tests and the installed-package tests, then creates the
   GitHub Release with `SHA256SUMS` and the generated AUR `PKGBUILD` attached.
5. After the release is published, update the AUR `rustxt-bin` package from
   the attached `PKGBUILD` if you maintain it.

A pre-release can be rehearsed with `workflow_dispatch`, which runs everything
except the publishing job.
