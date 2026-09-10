# Iced migration and footprint

Measured on 2026-09-10, branch `feat/iced-cross-platform`, rebased onto
`origin/main` at `8320c53` (the merged 0.4.1 hardening changes).

## Result

The default frontend is now `rustxt-iced`. The GTK crate is retained as an
explicit comparison target. Both use `rustxt-core` and the existing recovery
format. No notes or user configuration were modified during testing.

| Measurement | GTK before | Iced after | Change |
|---|---:|---:|---:|
| Stripped release executable | 4.38 MiB | 14.45 MiB | +10.08 MiB / 3.30× |
| Empty tab: RSS | 143.35 MiB | 21.80 MiB | −84.8% |
| Empty tab: PSS | 77.04 MiB | 16.06 MiB | −79.1% |
| Empty tab: private memory (USS) | 67.54 MiB | 14.29 MiB | −78.8% |
| ~1 MiB mixed-text file: RSS | 149.39 MiB | 29.78 MiB | −80.1% |
| ~1 MiB mixed-text file: PSS | 82.58 MiB | 23.85 MiB | −71.1% |
| ~1 MiB mixed-text file: private memory | 72.84 MiB | 22.02 MiB | −69.8% |

The executable comparison excludes external libraries. GTK relies on installed
GTK/libadwaita/GtkSourceView libraries; Iced statically links much of its GUI and
text stack. This is executable size, not a comparison of complete installation
sizes. The exact binaries are 4,589,160 and 15,154,256 bytes.

RSS counts every resident page mapped by the process, including shared libraries.
PSS apportions shared pages between processes. USS counts private clean and dirty
pages. These are settled process measurements, not peak allocation, GPU memory,
or total system memory including the display server/desktop portal.

## Method

- Same machine: x86_64 Linux, kernel `7.2.3-arch1-3`.
- Both built with the workspace release profile: thin LTO, one codegen unit,
  stripped symbols. The original GTK executable was preserved before migration.
- Xvfb display at 1280×1024, each application window explicitly resized to
  1000×700. GTK uses `GSK_RENDERER=cairo`; Iced uses `tiny-skia` with no WGPU backend.
- Separate temporary XDG and recovery directories for each run.
- One empty tab, then one file containing repeated English/Arabic lines totaling
  just over 1 MiB. No other instance of the measured frontend runs concurrently.
- Five seconds of warm-up after a visible window appears. Five samples of
  `/proc/PID/smaps_rollup`, 200 ms apart. Median within each run, then median across
  three fresh processes per workload.

Raw results, including binary SHA-256 hashes:
[GTK](benchmarks/gtk-linux.json) and [Iced](benchmarks/iced-linux.json).
The preserved GTK executable is in `target/footprint/rustxt-gtk` in this workspace;
the new executable is `target/release/rustxt-iced`.

To repeat on Linux:

```sh
xvfb-run -a -s '-screen 0 1280x1024x24' python3 tools/measure-footprint.py target/footprint/rustxt-gtk --output /tmp/gtk.json
xvfb-run -a -s '-screen 0 1280x1024x24' python3 tools/measure-footprint.py target/release/rustxt-iced --output /tmp/iced.json
```

These measurements establish the change on this Linux software-rendered workload.
They do not predict Windows/macOS memory, hardware-rendered GTK memory, or the
cost of hours of editing with large undo histories.

## Implementation

The frontend includes tabs, menus, file dialogs, keyboard shortcuts, find/replace,
settings, font selection by family, themes, zoom, printing, recent closed tabs,
recovery, and single-instance file forwarding. Command shortcuts adapt to macOS.

Undo records store changed spans rather than full document copies, with an
approximately 8 MiB per-tab budget (at least one edit is retained). History and
the recovery snapshot commit in one SQLite transaction. A fingerprint prevents
replaying history against text that has changed independently. Cursor positions
convert between GTK's character offsets and Iced's UTF-8 byte columns. Iced
viewport metadata is separate, so existing GTK recovery records remain readable.

The small local Iced patches are documented in [vendor/README.md](../vendor/README.md).
They correct RTL caret/selection geometry, avoid shaping the entire document
before the first layout, clear stale selections, and expose scroll restoration.
Their regression tests must remain enabled when updating Iced.

Shared-core portability changes cover Windows configuration/data paths and file
saves. Update checksums now use Rust SHA-256 instead of a `sha256sum` executable.
The old Unix tarball updater's integration tests remain Unix-specific. A flaky
checksum-corruption fixture was corrected so the replacement digit always differs.

## Validation and limits

- 52 core/editor/IPC and migration tests pass on Linux.
- Workspace formatting and Clippy with warnings denied pass.
- All 8 retained GTK end-to-end tests pass against the shared-core changes.
- Real X11 UI tests pass: typing, undo/redo, forced process termination and recovery,
  recovered undo history, tabs, closing/reopening notes, find selection/replacement,
  Arabic/English rendering, file forwarding, CRLF saves, external-change protection,
  settings, scroll recovery, and clean exit.
- Windows and macOS build/test/artifact jobs are configured in
  `.github/workflows/iced.yml`; they have not been run in this local session.
  Platform smoke testing, IME/accessibility testing, and macOS signing/notarization
  remain release qualification work.
- Native file dialogs require the desktop portal on Linux. The automated Xvfb
  tests cover file opening through command-line forwarding and existing-file saves;
  they do not exercise the system file picker or physical printing.
- Printing opens a local HTML print view in the default browser. The GTK-native
  print compositor is not carried over.
- The About dialog opens the release page. Automatic executable replacement is
  not enabled for this development frontend, so it cannot install a GTK release
  over itself.
- Menus are drawn inside the application; macOS global menu integration and Finder
  document-open events are not implemented. Opening via the CLI, in-app dialog,
  and file drop is supported.
- Existing GTK and Iced processes must not edit the same recovery database at the
  same time. The Iced instance lock coordinates Iced processes; older GTK builds
  do not participate in it.

Screenshots: [editor](screenshots/iced-editor.png), [settings](screenshots/iced-settings.png).
