"""Behaviour regressions found by exploratory testing. Run with xvfb-run -a.

    xvfb-run -a -s '-screen 0 1280x1024x24' python3 tools/iced-regressions-e2e.py BINARY OUTPUT

Each check launches a fresh sandboxed instance, so one failure cannot taint
the next, and leaves its artefacts under OUTPUT/<check>/. Every wait polls an
observable (session database, config file, screenshot); see e2e_common.py.
"""
import os
import re
import shutil
import sys
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2e_common import (  # noqa: E402
    RENDER_SETTLE, Harness, Image, InkMask, changed_fraction, distinct_colors, editor_region, resolve_binary,
    sandbox_env, stable_capture,
)

binary = resolve_binary(sys.argv[1])
output = Path(sys.argv[2] if len(sys.argv) > 2 else '/tmp/rustxt-iced-regressions')

MODAL_CHANGE_MINIMUM = 0.3
"""Fraction of window pixels that change when a modal opens or closes: the
40% black overlay touches every non-black pixel (see iced-e2e.py)."""
ZOOM_PACING = 1.0
"""Seconds between zoom chords. The regression was in the config watcher: the
app writes config.toml, the watcher sees its own write about a second later
and used to reload the file over a newer in-memory zoom. Pressing slowly
leaves the watcher time to fire between steps, which is the case that broke."""
DIALOG_HEIGHT_RATIO = 1.2
"""Minimum height of the startup error dialog listing two failures relative
to the dialog for one failure. The text cannot be read back, but failures are
joined with a blank line, so a second entry adds at least two 18 px lines to
the panel. The fixed parts (title, spacing, button, 24 px padding) are about
110 px and the single "missing.txt: Could not open file: No such file or
directory (os error 2)" message already wraps to two lines of the 520 px
panel, so one failure measures 174 px and two 230 px (ratio 1.32) at 100%
scale. A dialog that drops the second failure stays at the one-entry height
(ratio 1.0); 1.2 (209 px) is about 20 px clear of both. Heights are printed."""
SEAM_HALF_HEIGHT = 6
"""Rows above and below the menu/editor boundary in the seam strip. The menu
text ends well above the boundary and the first editor line starts below the
editor padding, so a correct frame has only flat colour here."""
SEAM_MAX_COLORS = 3
"""Menu bar background, the 1 px separator and the editor background. A
glyph fragment painted into the strip adds anti-aliased colours."""
TAB_TITLE_CROP = (40, 0, 580, 46)
"""Tab bar strip in logical px from crates/rustxt-iced/src/app/view.rs:
tabs are 216 px wide with 4 px spacing (tab 3 spans 440-656); x 40-620 skips
the first tab's dirty dot (x 10-24) and stops short of the third (selected)
tab's close icon at x 626+ and the "+" button that follows."""
TAB_TITLE_MAX_BAND = 22
"""Ink rows of a single title line at the 15 px UI size (about 12 rows). A
title wrapped onto two lines in the 34 px tab gives two bands or, if they
touch, one band of about 30 rows."""

seconds = {}


@contextmanager
def check(name, temp):
    """A fresh sandbox, Harness and output dir for one regression check."""
    root = Path(temp) / name
    root.mkdir()
    env = sandbox_env(root)
    started = time.monotonic()
    print(f'check: {name}', flush=True)
    with Harness(binary, output / name, root, env) as h:
        yield h, root
    seconds[name] = time.monotonic() - started


def launch(h, *args):
    h.app.launch(*args)
    h.app.wait_for(h.database.open_rows, 'Session database has no open document after launch')


def config_zoom(root):
    text = (root / 'config/rustxt/config.toml').read_text()
    match = re.search(r'^zoom\s*=\s*(\d+)', text, re.MULTILINE)
    return int(match.group(1)) if match else None


def dialog_height(h, dialog_shot, dismissed_shot):
    """Height in px of the modal panel in `dialog_shot`: within the editor
    rows (scanned on `dismissed_shot`, which has no overlay) take the middle
    third of the width, where the 520 px centred panel is the only thing that
    differs from the overlay-darkened editor background, and measure the one
    band of rows that differ from that background (sampled at the left edge,
    outside the panel, halfway down the editor)."""
    (x, top, width, height), _ = editor_region(h.app, dismissed_shot)
    darkened = Image.load(dialog_shot, crop=(x + 60, top + height // 2, 1, 1)).pixel(0, 0)
    # The panel is only ~10% per channel lighter than the darkened editor, so
    # the default 15% ink fuzz would see just its text; 5% sees the panel.
    mask = InkMask.load(dialog_shot, darkened, crop=(width // 3, top, width // 3, height), fuzz='5%')
    bands = mask.ink_bands()
    assert len(bands) == 1, f'Expected one solid dialog band in the editor centre, found {bands}'
    return bands[0][1] - bands[0][0] + 1


def startup_dialog(h, name, *args):
    """Launch with `args`, return the height of the startup error dialog and
    assert it was a modal (dismissing it changes most of the window)."""
    app = h.app
    app.launch(*args)
    dialog = app.wait_for(lambda: stable_capture(app, f'{name}-dialog'), 'Startup dialog did not render')
    app.key('Escape')
    dismissed = h.output / f'{name}-dismissed.png'
    app.wait_for(lambda: changed_fraction(dialog, app.capture(dismissed)) > MODAL_CHANGE_MINIMUM,
                 'Startup error dialog was not shown (dismissing it changed nothing)')
    app.wait_for(lambda: stable_capture(app, dismissed), 'Editor did not settle after dismissing the dialog')
    return dialog_height(h, dialog, dismissed)


with tempfile.TemporaryDirectory(prefix='rustxt-regressions-') as temp:
    with check('undo-by-word', temp) as (h, root):
        app, db = h.app, h.database
        launch(h)
        app.type_text('hello world again')
        app.wait_for(lambda: db.active_text() == 'hello world again', 'Typing not persisted')
        for expected in ('hello world ', 'hello ', ''):
            app.key('ctrl+z')
            app.wait_for(lambda: db.active_text() == expected, f'Undo did not step back to {expected!r}')

    with check('zoom-survives-watcher', temp) as (h, root):
        app = h.app
        launch(h)
        app.type_text('z')
        # Ctrl+wheel zoom is not covered: under Xvfb every xdotool wheel click
        # arrives as two wheel events, so one click zooms twice and the
        # expected value cannot be stated.
        for chord, expected in (('ctrl+minus', 90), ('ctrl+minus', 80), ('ctrl+minus', 70), ('ctrl+0', 100)):
            app.key(chord)
            app.wait_for(lambda: config_zoom(root) == expected, f'config.toml did not reach zoom = {expected} after {chord}')
            time.sleep(ZOOM_PACING)
            assert config_zoom(root) == expected, f'zoom = {expected} was reverted after {chord}: {config_zoom(root)}'

    with check('startup-errors-listed', temp) as (h, root):
        app, db = h.app, h.database
        h.step('reference: one missing path')
        one_entry = startup_dialog(h, 'one', root / 'missing.txt')
        app.stop()
        h.step('a good file, a directory and a missing path')
        good = root / 'good.txt'
        good.write_text('ok\n')
        directory = root / 'a-directory'
        directory.mkdir()
        shutil.rmtree(root / 'data')
        two_entries = startup_dialog(h, 'two', good, directory, root / 'missing.txt')
        rows = db.open_rows()
        assert len(rows) == 1 and db.active_file() == str(good), f'Expected only {good} open, got {rows}'
        print(f'  dialog height: one failure {one_entry} px, two failures {two_entries} px', flush=True)
        assert two_entries >= DIALOG_HEIGHT_RATIO * one_entry, (
            f'Dialog for two failures is {two_entries} px, one failure {one_entry} px: '
            f'expected at least {DIALOG_HEIGHT_RATIO}x, so not both failures are listed')

    with check('read-only-file', temp) as (h, root):
        app, db = h.app, h.database
        if os.geteuid() == 0:
            print('  skipped: root ignores file modes, so a read-only file cannot refuse a write', flush=True)
        else:
            locked = root / 'locked.txt'
            locked.write_text('keep\n')
            os.chmod(locked, 0o444)
            launch(h, locked)
            app.key('ctrl+End')
            app.type_text('edit')
            app.wait_for(lambda: db.active_text() == 'keep\nedit', 'Edit not persisted')
            time.sleep(RENDER_SETTLE)
            before = app.capture('before-save')
            app.key('ctrl+s')
            app.wait_for(lambda: changed_fraction(before, app.capture('refused')) > MODAL_CHANGE_MINIMUM,
                         'Save to a read-only file showed no error dialog')
            assert locked.read_text() == 'keep\n', f'Read-only file was written: {locked.read_text()!r}'
            app.key('Escape')
            app.type_text('!')
            app.wait_for(lambda: db.active_text() == 'keep\nedit!', 'Editing after the refused save failed')
            assert locked.read_text() == 'keep\n', 'Read-only file was written after the dialog'

    with check('menu-bar-seam', temp) as (h, root):
        app, db = h.app, h.database
        long = root / 'long.txt'
        long.write_text(''.join(f'line {i:03d} fdfdffghfdfdffgh\n' for i in range(90)))
        launch(h, long)
        app.key('ctrl+End')
        app.wait_for(lambda: db.active_scroll_line() > 0, 'Ctrl+End did not scroll')
        for _ in range(12):
            app.key('Up')
        geometry = app.geometry()
        app.mouse_move(geometry['WIDTH'] * 0.4, geometry['HEIGHT'] * 0.5)
        app.click(5)
        app.click(4)
        for _ in range(12):
            app.key('Down')
        for _ in range(6):
            app.type_text('x')
            app.wait_for(lambda: db.active_text().endswith('x'), 'Typed marker not persisted')
            app.key('BackSpace')
            app.wait_for(lambda: not db.active_text().endswith('x'), 'Backspace not persisted')
        shot = app.wait_for(lambda: stable_capture(app, 'after-edits'), 'Editor did not settle after the edits')
        (_, top, width, _), _ = editor_region(app, shot)
        strip = (10, top - SEAM_HALF_HEIGHT, width - 20, 2 * SEAM_HALF_HEIGHT)
        colors = distinct_colors(shot, strip)
        print(f'  seam strip {strip}: {colors} distinct colours', flush=True)
        assert colors <= SEAM_MAX_COLORS, (
            f'{colors} distinct colours in the menu/editor seam {strip} (flat chrome has at most '
            f'{SEAM_MAX_COLORS}): text was painted over the menu bar')

    with check('long-tab-titles', temp) as (h, root):
        app, db = h.app, h.database
        files = [root / f'a-very-long-document-name-number-{n}.txt' for n in range(3)]
        for path in files:
            path.write_text('x\n')
        launch(h, *files)
        app.wait_for(lambda: len(db.open_rows()) == 3 and db.active_file() == str(files[2]), 'Three tabs did not open')
        geometry = app.geometry()
        app.mouse_move(geometry['WIDTH'] * 0.5, geometry['HEIGHT'] * 0.5)  # no tab hover state
        shot = app.wait_for(lambda: stable_capture(app, 'tabs'), 'Tab bar did not settle')
        background = Image.load(shot, crop=(5, 5, 1, 1)).pixel(0, 0)
        bands = InkMask.load(shot, background, crop=TAB_TITLE_CROP).ink_bands()
        print(f'  tab title ink bands: {bands}', flush=True)
        assert len(bands) == 1, f'Tab titles should be one row of text, found ink bands {bands}'
        assert bands[0][1] - bands[0][0] + 1 <= TAB_TITLE_MAX_BAND, f'Tab title ink band {bands[0]} is taller than one line'

print('PASS: ' + ', '.join(f'{name} ({elapsed:.1f}s)' for name, elapsed in seconds.items()))
