# Iced migration and footprint

Measured on 2026-09-10, branch `feat/iced-cross-platform`, rebased onto
`origin/main` at `8320c53` (the merged 0.4.1 hardening changes).

## Initial migration result

The default frontend is now `rustxt-iced`. The GTK crate served as the
comparison target for these measurements; it was removed from the tree in
0.6.0 and is preserved at git tag `v0.4.1-gtk` (see [gtk.md](gtk.md)). Both
frontends use `rustxt-core` and the same recovery format. No notes or user
configuration were modified during testing.

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

- Both frontends measured on the same x86_64 Linux machine. The raw JSON files
  record the host, kernel, and binary hashes of the recorded run.
- Both built with the workspace release profile: thin LTO, one codegen unit,
  stripped symbols. The GTK executable is built from tag `v0.4.1-gtk`
  (`cargo build --release -p rustxt` in a checkout of that tag, with the GTK 4,
  libadwaita, and GtkSourceView 5 development packages installed); the Iced
  executable is `target/release/rustxt-iced` from the current checkout.
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
The follow-up below records a specific renderer-fix snapshot, not every
subsequent release build.

To repeat on Linux, build the GTK binary from the tag and point the script at
both executables (`tools/measure-footprint.py` needs Python 3, `xvfb-run`, and
`xdotool`):

```sh
git worktree add ../rustxt-gtk v0.4.1-gtk
(cd ../rustxt-gtk && cargo build --release -p rustxt)
cargo build --release -p rustxt-iced
xvfb-run -a -s '-screen 0 1280x1024x24' python3 tools/measure-footprint.py ../rustxt-gtk/target/release/rustxt --output /tmp/gtk.json
xvfb-run -a -s '-screen 0 1280x1024x24' python3 tools/measure-footprint.py target/release/rustxt-iced --output /tmp/iced.json
```

These measurements establish the change on this Linux software-rendered workload.
They do not predict Windows/macOS memory, hardware-rendered GTK memory, or the
cost of hours of editing with large undo histories.

## UI and renderer follow-up

The updated build restores the tab/menu arrangement, desktop UI font, vector
controls, grouped Settings rows, font selection, and Find/Replace layout and
focus behavior. It is compared against the installed `/usr/bin/rustxt` 0.4.1.
Exact pixel parity is not claimed: native GTK font dialogs, some widget metrics,
text rasterization and search highlighting still differ.

A software-renderer bug caused hovering over menus and Settings to repeatedly
paint unclipped shadows over unchanged pixels. The vendored fix clips shadows
and limits their rasterization to the damaged area. A regression test failed
before the fix and passes afterward at 100%, 125%, 150%, and 200% scale; a real
window test checks 33 hover redraws at 125%. See `vendor/README.md`.

At the renderer-fix snapshot, the release binary was **17,272,304 bytes
(16.47 MiB)**. Median RSS is **23.61 MiB empty**
and **30.74 MiB with the ~1 MiB file**, compared with the original GTK
143.35 / 149.39 MiB. The initial table above describes the earlier binary.
[Follow-up raw measurements](benchmarks/iced-ui-linux.json) include that snapshot’s
binary hash. Other Iced windows were open during portions of this follow-up,
so its PSS/USS must not be compared with the isolated initial measurements.

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
saves. The in-app updater inherited from the GTK frontend was never enabled in
Iced and is removed from the core in 0.6.0; the About dialog links to the
releases page instead.

## Validation and limits

- At the time of the migration, 52 core/editor/IPC and migration tests passed
  on Linux.
- Workspace formatting and Clippy with warnings denied pass.
- At the time of the migration, all 8 GTK end-to-end tests (now at tag
  `v0.4.1-gtk`) passed against the shared-core changes.
- Real X11 UI tests (`tools/iced-e2e.py`) pass: typing, undo/redo, forced process
  termination and recovery, recovered undo history, tabs, closing/reopening
  notes, find selection/replacement, file forwarding, CRLF saves,
  external-change protection, settings, scroll recovery, and clean exit. An
  Arabic/English document is opened and a screenshot (`editor.png`) is captured
  for manual review; the harness makes no assertion about how it is rendered.
- Native CI validates Linux, Windows, Apple Silicon macOS, and Intel macOS with
  formatting, Clippy, core/editor tests, renderer regressions, and release builds.
  Native startup smoke tests load Unicode filenames, forward a second launch into
  the first instance, and restore the session after terminating the process.
- Arch, Debian 13, and Fedora 42 packages are installed in clean containers and
  pass the same startup/forwarding/recovery check. Runtime-loaded X11/Wayland
  libraries are explicitly declared because ELF dependency scanning misses them.
- macOS bundles are ad-hoc signed and verified, but not Apple-notarized. Windows
  ZIPs include the C runtime. IME and accessibility behavior still require manual
  platform validation.
- Native file dialogs require the desktop portal on Linux. The automated Xvfb
  tests cover file opening through command-line forwarding and existing-file saves;
  they do not exercise the system file picker or physical printing.
- Printing opens a local HTML print view in the default browser. The GTK-native
  print compositor is not carried over.
- The About dialog opens the release page. Automatic executable replacement is
  not enabled for the Iced frontend.
- Menus are drawn inside the application; macOS global menu integration and Finder
  document-open events are not implemented. Opening via the CLI, in-app dialog,
  and file drop is supported.
- An installed 0.4.x GTK build and an Iced build must not edit the same recovery
  database at the same time. The Iced instance lock coordinates Iced processes;
  the 0.4.x builds do not participate in it.

The release qualification run is recorded in
[the cross-platform package workflow](https://github.com/tsubaie/RusTXT/actions/runs/34477781286)
and [the lint/test matrix](https://github.com/tsubaie/RusTXT/actions/runs/34477781159).

Screenshots: [editor](screenshots/iced-editor.png), [settings](screenshots/iced-settings.png).
