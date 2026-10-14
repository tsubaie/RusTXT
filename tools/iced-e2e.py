"""Real keyboard/mouse and crash recovery checks. Run with xvfb-run -a.

    xvfb-run -a -s '-screen 0 1280x1024x24' python3 tools/iced-e2e.py BINARY OUTPUT

Every step waits for an observable effect (session database rows, window
state, screenshot content) rather than sleeping; see tools/e2e_common.py.
"""
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2e_common import (  # noqa: E402
    RENDER_SETTLE, Harness, Image, InkMask, changed_fraction, resolve_binary, sandbox_env, sha256,
)

binary = resolve_binary(sys.argv[1])
output = Path(sys.argv[2] if len(sys.argv) > 2 else '/tmp/rustxt-iced-e2e')

# Screenshot thresholds. The editor is cropped to the text area derived from
# the window geometry, so the numbers hold at any window size and scale:
#   EDITOR_INK_MINIMUM   ink pixels (colour further than 15% from the editor
#                        background) for the two rendered lines; at 15 px the
#                        Arabic line alone produces ~240 and the English line
#                        ~600, a blank editor 0 and a lone caret < 40.
#   RTL_SIDE_FRACTION    share of a line's ink that must sit in the half of the
#                        editor its direction starts from: an RTL-only line is
#                        right-aligned (cosmic-text aligns to the paragraph
#                        direction), so all glyph ink is in the right half and
#                        at most a caret can be on the left.
#   MODAL_CHANGE_MINIMUM fraction of window pixels that change when a modal
#                        opens: the 40% black overlay changes every non-black
#                        pixel and the Settings panel alone covers 64% of a
#                        1000 px wide window.
EDITOR_INK_MINIMUM = 150
RTL_SIDE_FRACTION = 0.9
MODAL_CHANGE_MINIMUM = 0.3

ARABIC = 'مرحبا بالعالم\nEnglish text 👋\n'


def editor_region(app, shot):
    """(x, y, width, height) of the text area, from the window geometry and
    the screenshot: scan the centre column from both ends until the editor
    background (the colour at the window centre, which stays blank because
    the RTL line is right-aligned and the LTR line left-aligned) is reached;
    the rows above are the tab and menu bars, the rows below the status bar."""
    geometry = app.geometry()
    width, height = geometry['WIDTH'], geometry['HEIGHT']
    column = Image.load(shot, crop=(width // 2, 0, 1, height))
    background = column.pixel(0, height // 2)
    top = next(y for y in range(height) if column.pixel(0, y) == background)
    bottom = next(y for y in range(height - 1, -1, -1) if column.pixel(0, y) == background)
    assert 0.05 * height < top < 0.3 * height, f'Editor top edge {top} not plausible for a {height} px window'
    assert bottom > 0.7 * height, f'Editor bottom edge {bottom} not plausible for a {height} px window'
    return (0, top, width, bottom - top + 1), background


def editor_text_check(app, shot):
    """Return the ink mask of the editor when both lines have rendered, else None."""
    region, background = editor_region(app, shot)
    mask = InkMask.load(shot, background, crop=region)
    if mask.count() < EDITOR_INK_MINIMUM or len(mask.ink_bands()) < 2:
        return None
    return mask


def assert_bidi_layout(mask):
    """Crude RTL check: the first line (Arabic only) must be right-aligned and
    the second (English) left-aligned, judged by where each line's ink sits."""
    half = mask.width // 2
    bands = mask.ink_bands()
    for index, (band, side) in enumerate(zip(bands[:2], ('right', 'left'))):
        rows = [mask.row(y) for y in range(band[0], band[1] + 1)]
        total = sum(row.count(b'\xff') for row in rows)
        on_side = sum((row[half:] if side == 'right' else row[:half]).count(b'\xff') for row in rows)
        assert on_side >= RTL_SIDE_FRACTION * total, (
            f'Line {index + 1} should be {side}-aligned: {on_side}/{total} ink pixels on the {side} '
            f'(rows {band[0]}-{band[1]} of the editor crop, width {mask.width})')


with tempfile.TemporaryDirectory(prefix='rustxt-iced-e2e-') as temp:
    root = Path(temp)
    env = sandbox_env(root)
    with Harness(binary, output, root, env) as h:
        app, db = h.app, h.database

        def launch():
            window = app.launch()
            app.wait_for(db.open_rows, 'Session database has no open document after launch')
            return window

        h.step('typing, undo and redo')
        launch()
        app.type_text('Recovery survives a crash.\nSecond line.')
        app.wait_for(lambda: db.active_text() == 'Recovery survives a crash.\nSecond line.', 'Typing not persisted')
        app.key('ctrl+z')
        app.wait_for(lambda: db.active_text() == 'Recovery survives a crash.\nSecond line', 'Undo failed')
        app.key('ctrl+shift+z')
        app.wait_for(lambda: db.active_text().endswith('line.'), 'Redo failed')

        h.step('crash recovery (SIGKILL) with undo history')
        app.kill()
        launch()
        app.wait_for(lambda: db.active_text() == 'Recovery survives a crash.\nSecond line.', 'Crash restore failed')
        app.key('ctrl+z')
        app.wait_for(lambda: db.active_text().endswith('line'), 'Recovered undo history failed')
        app.key('ctrl+shift+z')
        app.wait_for(lambda: db.active_text().endswith('line.'), 'Recovered redo history failed')

        h.step('tabs: new, close, reopen')
        app.key('ctrl+t')
        app.type_text('A second unsaved note')
        app.wait_for(lambda: db.active_text() == 'A second unsaved note', 'Second tab failed')
        app.key('ctrl+w')
        app.wait_for(lambda: db.active_text().startswith('Recovery'), 'Close tab failed')
        app.key('ctrl+shift+t')
        app.wait_for(lambda: db.active_text() == 'A second unsaved note', 'Reopen failed')

        h.step('find and replace')
        app.key('ctrl+f')
        app.type_text('second')
        app.key('Return')
        app.key('ctrl+h')
        app.type_text('restored')
        app.key('Return')
        app.wait_for(lambda: db.active_text() == 'A restored unsaved note', 'Find selection/replacement failed')
        app.key('Escape')

        h.step('single-instance forwarding of an Arabic CRLF file')
        sample = root / 'arabic.txt'
        original = ARABIC.replace('\n', '\r\n').encode()
        sample.write_bytes(original)
        app.run_once(sample)
        app.wait_for(lambda: len(db.open_rows()) == 3, 'Single-instance forwarding failed')
        app.wait_for(lambda: db.active_file() == str(sample), 'Forwarded file is not the active tab')
        # A clean file-backed document stores no text; the file is authoritative.
        app.wait_for(lambda: db.document(sample) == ('', 0, sha256(original)), 'Forwarded document path/content mismatch')

        h.step('editor screenshot: text rendered, RTL line right-aligned')
        editor_shot = output / 'editor.png'
        mask = app.wait_for(lambda: editor_text_check(app, app.capture(editor_shot)), 'Editor did not render the forwarded text')
        assert_bidi_layout(mask)
        print(f'  editor ink: {mask.count()} px in {len(mask.ink_bands())} lines', flush=True)

        h.step('save preserves CRLF')
        app.key('ctrl+End')
        app.type_text('end')
        app.key('ctrl+s')
        app.wait_for(lambda: sample.read_bytes().endswith(b'\r\nend'), 'Save did not preserve CRLF')
        saved = sample.read_bytes()
        assert saved == original + b'end', f'Saved bytes unexpected: {saved!r}'
        app.wait_for(lambda: db.document(sample) == ('', 0, sha256(saved)), 'Save was not recorded as clean with the new fingerprint')

        h.step('external change is never overwritten')
        external = b'Changed by another program'
        sample.write_bytes(external)
        app.type_text('!')
        edited = ARABIC + 'end!'
        app.wait_for(lambda: db.document(sample) == (edited, 1, sha256(saved)), 'Edit after the external change was not snapshotted')
        app.key('ctrl+s')
        # The refusal is visible as the error modal (a 40% black overlay plus a
        # dialog), so poll screenshots until the window changed substantially.
        conflict_shot = output / 'conflict.png'
        app.wait_for(lambda: changed_fraction(editor_shot, app.capture(conflict_shot)) > MODAL_CHANGE_MINIMUM,
                     'Save-refused dialog did not appear')
        assert sample.read_bytes() == external, 'External changes overwritten'
        assert db.document(sample) == (edited, 1, sha256(saved)), f'Refused save altered the snapshot: {db.document(sample)}'
        app.key('Escape')
        # Events are processed in order: once this marker is persisted, the
        # refused save and the dismissal were handled before it.
        app.type_text('?')
        app.wait_for(lambda: db.document(sample) == (edited + '?', 1, sha256(saved)), 'Editing after dismissing the dialog failed')
        assert sample.read_bytes() == external, 'External changes overwritten after dismissal'

        h.step('settings dialog')
        time.sleep(RENDER_SETTLE)
        reference = app.capture('before-settings')
        app.key('ctrl+comma')
        settings_shot = output / 'settings.png'
        app.wait_for(lambda: changed_fraction(reference, app.capture(settings_shot)) > MODAL_CHANGE_MINIMUM, 'Settings dialog did not appear')
        app.key('Escape')
        app.wait_for(lambda: changed_fraction(reference, app.capture('after-settings')) < MODAL_CHANGE_MINIMUM, 'Settings dialog did not close')

        h.step('scroll position survives a crash')
        long_file = root / 'long.txt'
        long_file.write_text('A line of text for scrolling.\n' * 1000)
        app.run_once(long_file)
        app.wait_for(lambda: len(db.open_rows()) == 4, 'Long file did not open')
        app.wait_for(lambda: db.active_file() == str(long_file), 'Long file is not the active tab')
        app.wait_for(lambda: db.active_scroll_line() == 0, 'Long file view state not recorded')
        geometry = app.geometry()
        app.mouse_move(geometry['WIDTH'] * 0.4, geometry['HEIGHT'] * 0.45)
        app.click(5, repeat=12)
        before_scroll = app.wait_for(lambda: db.active_scroll_line() if db.active_scroll_line() > 10 else None, 'Scroll position not saved')
        app.kill()
        launch()
        app.wait_for(lambda: db.active_file() == str(long_file), 'Long file not restored as the active tab')
        app.mouse_move(geometry['WIDTH'] * 0.4, geometry['HEIGHT'] * 0.45)
        app.click(5)
        app.wait_for(lambda: db.active_scroll_line() > before_scroll, 'Viewport did not restore after restart')

        h.step('clean exit')
        app.key('ctrl+shift+w')
        code = app.wait_exit(timeout=15)
        assert code == 0, f'App exited with {code}'
        print('PASS: typing, undo/redo, crash recovery, tabs, reopen, find selection, file forwarding, '
              'RTL rendering, CRLF save, external-change protection, settings, scroll recovery, clean exit')
