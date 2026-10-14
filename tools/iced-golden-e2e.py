"""Golden screenshots of five UI states at 100% and 125% scale. Run with xvfb-run -a.

    xvfb-run -a -s '-screen 0 1600x1200x24' python3 tools/iced-golden-e2e.py BINARY OUTPUT [--update]

Each state is captured in a sandbox whose every known source of variance is
pinned (see SANDBOX, CONFIG, DOCUMENT and tools/goldens/README.md) and
compared with tools/goldens/<state>@<scale>.png. With --update the goldens
are rewritten instead. The goldens are made on the CI Ubuntu image, whose
fonts differ from a developer machine's, so refresh them there
(workflow_dispatch with update_goldens) rather than locally.
"""
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2e_common import (  # noqa: E402
    CARET_NOISE, IMAGEMAGICK_COMPARE, Harness, Image, changed_fraction, image_size, resolve_binary, sandbox_env,
    stable_capture, xdo,
)

binary = resolve_binary(sys.argv[1])
output = Path(sys.argv[2])
update = '--update' in sys.argv[3:]
GOLDENS = Path(__file__).resolve().parent / 'goldens'
HOW_TO_GENERATE = ('run the CI workflow manually with update_goldens=true and commit the "goldens" artifact '
                   '(or locally: make goldens-update; see tools/goldens/README.md)')

SCALES = (1.0, 1.25)
WINDOW = (1000, 700)
"""Logical window size, requested explicitly so the layout is the same
whatever the Xvfb screen size."""
STATES = ('error-dialog', 'editor', 'file-menu', 'settings', 'find-bar')
SANDBOX = Path(tempfile.gettempdir()) / 'rustxt-golden'
"""Fixed path, not a random temporary directory: the Settings panel prints
the configuration and themes paths, and the error dialog names the file, so
the sandbox must be at the same path on every machine."""
CONFIG = '''[appearance]
theme = "dark"
zoom = 100

[editor]
word_wrap = true
font = "DejaVu Sans Mono 11"

[window]
status_bar = true
title_bar = "hide"
'''
"""theme="dark" rather than "auto" (which follows the system setting); the
font is named (fonts-dejavu-core on the CI image) rather than left to
fc-match; the status bar is kept, it shows only the cursor position,
character count, zoom, line ending and encoding, nothing live."""
DOCUMENT = '''The quick brown fox jumps over the lazy dog.
مرحبا بالعالم، هذا سطر عربي في المستند.
A second English line keeps the editor busy, and the third line is Arabic:
النص العربي يُعرض من اليمين إلى اليسار.
The end of the document, where the find bar counts the matches of "the".
'''
FIND_QUERY = 'the'
TILING_COMPOSITOR_VARIABLES = (
    'HYPRLAND_INSTANCE_SIGNATURE', 'SWAYSOCK', 'NIRI_SOCKET', 'XDG_CURRENT_DESKTOP', 'XDG_SESSION_DESKTOP')
"""Removed from the app's environment: the Settings panel's title bar caption
says whether a tiling compositor was detected from these (desktop.rs), and
a developer's desktop must not leak into the capture."""

TOLERANCE_THRESHOLD = '3.3%'
"""ImageMagick threshold for a per-channel tolerance of 8 units: 8/255 is
3.137% and 9/255 is 3.529%, so a difference of 8 is accepted on both Q8 and
Q16 builds and 9 is not. Absorbs the ±1..2 unit rounding differences between
blending paths (see e2e_common.color_close)."""
MAX_DIFFERING = 0.005
"""Fraction of pixels allowed to differ by more than the tolerance: 0.5%,
about 3,500 px of a 1000x700 window. The blinking caret and a glyph's
hinting at a slightly different sub-pixel offset stay far below this; a
moved panel, a missing line of text or a colour change exceed it."""
MODAL_CHANGE_MINIMUM = 0.3
"""Fraction of window pixels a modal changes (its 40% black overlay)."""
POPUP_CHANGE_MINIMUM = 0.005
"""Fraction of window pixels the File menu or the find bar changes: both
cover a few percent of the window, far above caret blink noise."""


def physical(scale, x, y):
    return round(x * scale), round(y * scale)


def write_sandbox():
    shutil.rmtree(SANDBOX, ignore_errors=True)
    config = SANDBOX / 'config/rustxt'
    config.mkdir(parents=True)
    (config / 'config.toml').write_text(CONFIG)
    document = SANDBOX / 'golden.txt'
    document.write_text(DOCUMENT)
    return document, SANDBOX / 'missing.txt'


def golden_path(state, scale):
    return GOLDENS / f'{state}@{scale}.png'


def status_bar_strip(path, scale):
    """Raw pixels of the status bar, where the cursor position is printed."""
    width, height = image_size(path)
    top = height - physical(scale, 0, 30)[1]
    return Image.load(path, crop=(0, top, width, height - top)).data


def compare(state, scale, actual, failures):
    golden = golden_path(state, scale)
    if update:
        shutil.copyfile(actual, golden)
        print(f'  wrote {golden.relative_to(GOLDENS.parent.parent)} {image_size(actual)}', flush=True)
        return
    actual_size, golden_size = image_size(actual), image_size(golden)
    if actual_size != golden_size:
        failures.append(f'{golden.name}: size {actual_size} differs from golden {golden_size}')
        shutil.copyfile(actual, output / f'{state}@{scale}.actual.png')
        return
    differing = changed_fraction(golden, actual, threshold=TOLERANCE_THRESHOLD)
    print(f'  {golden.name}: {differing * 100:.3f}% of pixels differ by more than 8 units', flush=True)
    if differing <= MAX_DIFFERING:
        return
    actual_copy = output / f'{state}@{scale}.actual.png'
    diff = output / f'{state}@{scale}.diff.png'
    shutil.copyfile(actual, actual_copy)
    # compare exits 1 when the images differ, which is the case here.
    subprocess.run([*IMAGEMAGICK_COMPARE, '-fuzz', '4%', '-highlight-color', 'red', str(golden), str(actual), str(diff)],
                   timeout=60, stderr=subprocess.DEVNULL)
    failures.append(f'{golden.name}: {differing * 100:.2f}% of pixels differ (limit {MAX_DIFFERING * 100}%); '
                    f'see {actual_copy} and {diff}')


def capture_states(scale, failures):
    document, missing = write_sandbox()
    env = sandbox_env(SANDBOX, scale=scale)
    for variable in TILING_COMPOSITOR_VARIABLES:
        env.pop(variable, None)
    with Harness(binary, output / f'scale-{scale}', SANDBOX, env) as h:
        app, db = h.app, h.database

        def settled(name, changed_from=None, minimum=0.0):
            """Stable capture `name` that differs from `changed_from` by more
            than `minimum` of the window, polled until the state has rendered."""
            def check():
                shot = stable_capture(app, name)
                if shot is None:
                    return None
                if changed_from is not None and changed_fraction(changed_from, shot) <= minimum:
                    return None
                return shot
            return app.wait_for(check, f'{name} did not render at scale {scale}')

        def restored(name, reference):
            app.wait_for(lambda: changed_fraction(reference, app.capture(name)) <= CARET_NOISE, f'{name}: editor not restored')

        h.step(f'launch at scale {scale} with a missing file')
        app.launch(document, missing)
        app.wait_for(db.open_rows, 'Session database has no open document after launch')
        app.resize(*physical(scale, *WINDOW))
        screen_width, screen_height = map(int, xdo(env, 'getdisplaygeometry').split())
        xdo(env, 'mousemove', screen_width - 1, screen_height - 1)  # no hover state anywhere

        h.step('error dialog')
        dialog = settled('error-dialog')
        app.key('Escape')
        dismissed = settled('dismissed', changed_from=dialog, minimum=MODAL_CHANGE_MINIMUM)
        compare('error-dialog', scale, dialog, failures)

        h.step('editor with the cursor on line 3, column 5')
        # Line 3 is English: on the RTL line 2, Right moves the caret
        # visually right, which is towards the line start, and the column
        # would stay at 1.
        for chord in ('ctrl+Home', 'Down', 'Down', 'Right', 'Right', 'Right', 'Right'):
            app.key(chord)
        reference = status_bar_strip(dismissed, scale)

        def cursor_moved():
            shot = stable_capture(app, 'editor')
            return shot if shot and status_bar_strip(shot, scale) != reference else None

        editor = app.wait_for(cursor_moved, 'Status bar did not show the moved cursor')
        compare('editor', scale, editor, failures)

        h.step('File menu')
        app.key('alt+f')
        compare('file-menu', scale, settled('file-menu', changed_from=editor, minimum=POPUP_CHANGE_MINIMUM), failures)
        app.key('Escape')
        restored('menu-closed', editor)

        h.step('Settings panel')
        app.key('ctrl+comma')
        compare('settings', scale, settled('settings', changed_from=editor, minimum=MODAL_CHANGE_MINIMUM), failures)
        app.key('Escape')
        restored('settings-closed', editor)

        h.step('find bar with a query and its "1 of N" match count')
        app.key('ctrl+Home')  # Return then selects the first match: "1 of N"
        app.key('ctrl+f')
        app.type_text(FIND_QUERY)
        app.key('Return')
        compare('find-bar', scale, settled('find-bar', changed_from=editor, minimum=POPUP_CHANGE_MINIMUM), failures)
        app.key('Escape')

        h.step('clean exit')
        app.key('ctrl+shift+w')
        code = app.wait_exit(timeout=15)
        assert code == 0, f'App exited with {code}'


output.mkdir(parents=True, exist_ok=True)
if not update:
    missing = [golden_path(state, scale) for scale in SCALES for state in STATES if not golden_path(state, scale).exists()]
    if missing:
        print('FAIL: missing golden screenshots:\n  ' + '\n  '.join(str(path) for path in missing)
              + f'\nTo generate them, {HOW_TO_GENERATE}.', flush=True)
        sys.exit(2)
GOLDENS.mkdir(exist_ok=True)
started = time.monotonic()
failures = []
for scale in SCALES:
    capture_states(scale, failures)
shutil.rmtree(SANDBOX, ignore_errors=True)
if failures:
    print('FAIL: golden screenshot mismatch\n  ' + '\n  '.join(failures), flush=True)
    sys.exit(1)
verb = 'updated' if update else 'match'
print(f'PASS: {len(STATES) * len(SCALES)} golden screenshots {verb} in {time.monotonic() - started:.1f}s '
      f'(tolerance 8/255 per channel, {MAX_DIFFERING * 100}% of pixels)')
