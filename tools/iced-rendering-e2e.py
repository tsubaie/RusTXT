"""Check panel colors during repeated hover redraws at fractional display scale.

Run: xvfb-run -a -s '-screen 0 1600x1200x24' python3 tools/iced-rendering-e2e.py BINARY OUTPUT

Guards against the hover regression where redrawing a menu or the Settings
panel while the pointer moves over its items tinted the panel background.
All positions are logical pixels relative to the window (or the Settings
panel), converted with the display scale, and read back from screenshots via
ImageMagick; see tools/e2e_common.py.
"""
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2e_common import Harness, Image, changed_fraction, color_close, resolve_binary, sandbox_env  # noqa: E402

binary = resolve_binary(sys.argv[1])
output = Path(sys.argv[2])
SCALE = 1.25
WINDOW = (1000, 700)
"""Logical window size; the app's default, requested explicitly so the
layout below is the same whatever the Xvfb screen size."""
MENU_COLOR = (49, 50, 68)       # theme menu="#313244"
CHROME_COLOR = (22, 22, 34)     # theme chrome="#161622"
COLOR_TOLERANCE = 2
"""Per channel; see e2e_common.color_close for why exact equality is wrong."""

# Layout facts from crates/rustxt-iced/src/app.rs (logical px):
MENU_BAR_BOTTOM = 79            # tab bar 47 + menu bar 31 + separator
EDIT_MENU_LEFT = 50             # dropdown left edge for the Edit menu
EDIT_MENU_WIDTH = 204
EDIT_ITEM_ROWS = (242, 319, 351, 415, 492)   # Paste, Find, Find next, Replace, Select all
SETTINGS_PANEL_WIDTH = 640      # centred horizontally, full height below 786
SETTINGS_PROBE = (10, 210)      # inside the panel's left margin, below the header
SETTINGS_HOVER = ((550, 170), (550, 275), (520, 360), (570, 450), (520, 540))  # pick lists, zoom, font, toggles
MODAL_CHANGE_MINIMUM = 0.3
HOVER_SETTLE = 0.12
"""Seconds between a pointer move and the screenshot that samples the frame
it caused. Nothing persists on hover, so there is no state to poll; Iced
redraws on the frame after the cursor event, and Xvfb has no vsync, so this
covers several frames. The menu loop additionally asserts that hovered items
were highlighted, proving the redraws did happen."""


with tempfile.TemporaryDirectory(prefix='rustxt-rendering-') as temp:
    root = Path(temp)
    env = sandbox_env(root, scale=SCALE)
    config = root / 'config/rustxt'
    (config / 'themes').mkdir(parents=True)
    (config / 'config.toml').write_text('[appearance]\ntheme="parity"\n[window]\ntitle_bar="hide"\n')
    (config / 'themes/parity.toml').write_text(
        'mode="dark"\nbackground="#1e1e2e"\nforeground="#cdd6f4"\nchrome="#161622"\nmenu="#313244"\naccent="#89b4fa"\n')

    with Harness(binary, output, root, env) as h:
        app = h.app

        def physical(x, y):
            return round(x * SCALE), round(y * SCALE)

        def hover(x, y):
            app.mouse_move(*physical(x, y))

        def probe(name, x, y):
            """Colour at logical window position (x, y) in a fresh screenshot."""
            px, py = physical(x, y)
            return Image.load(app.capture(name), crop=(px, py, 1, 1)).pixel(0, 0)

        def expect(name, x, y, expected, description):
            value = probe(name, x, y)
            assert color_close(value, expected, COLOR_TOLERANCE), f'{description}: {value} != {expected} (±{COLOR_TOLERANCE})'
            return value

        h.step('launch and size the window')
        app.launch()
        app.wait_for(h.database.open_rows, 'Session database has no open document after launch')
        app.resize(*physical(*WINDOW))
        geometry = app.geometry()
        width, height = geometry['WIDTH'] / SCALE, geometry['HEIGHT'] / SCALE
        assert (round(width), round(height)) == WINDOW, f'Unexpected logical window size {width}x{height}'
        idle = app.capture('idle')

        h.step('Edit menu stays its colour through hover redraws')
        app.key('alt+e')
        menu_probe = (EDIT_MENU_LEFT + 10, MENU_BAR_BOTTOM + 10)
        app.wait_for(lambda: color_close(probe('menu-before', *menu_probe), MENU_COLOR, COLOR_TOLERANCE), 'Edit menu did not open')
        item_x = EDIT_MENU_LEFT + EDIT_MENU_WIDTH // 3
        highlighted = 0
        for frame, y in enumerate((EDIT_ITEM_ROWS + EDIT_ITEM_ROWS[:1]) * 3):
            hover(item_x, y)
            time.sleep(HOVER_SETTLE)
            shot = app.capture('menu-after')
            value = Image.load(shot, crop=(*physical(*menu_probe), 1, 1)).pixel(0, 0)
            assert color_close(value, MENU_COLOR, COLOR_TOLERANCE), f'Menu changed colour at frame {frame}: {value} != {MENU_COLOR} (±{COLOR_TOLERANCE})'
            if not color_close(Image.load(shot, crop=(*physical(item_x, y), 1, 1)).pixel(0, 0), MENU_COLOR, COLOR_TOLERANCE):
                highlighted += 1
        assert highlighted, 'No hovered menu item was ever highlighted: hover redraws did not happen'
        app.key('Escape')
        app.wait_for(lambda: changed_fraction(idle, app.capture('menu-closed')) < 0.05, 'Edit menu did not close')

        h.step('Settings panel stays its colour through hover redraws')
        app.key('ctrl+comma')
        panel_left = (width - SETTINGS_PANEL_WIDTH) / 2
        settings_probe = (panel_left + SETTINGS_PROBE[0], SETTINGS_PROBE[1])
        app.wait_for(lambda: changed_fraction(idle, app.capture('settings-before')) > MODAL_CHANGE_MINIMUM, 'Settings did not open')
        expect('settings-before', *settings_probe, CHROME_COLOR, 'Settings panel probe is not panel chrome')
        for frame, (x, y) in enumerate(SETTINGS_HOVER * 3):
            hover(panel_left + x, y)
            time.sleep(HOVER_SETTLE)
            expect('settings-after', *settings_probe, CHROME_COLOR, f'Settings changed colour at frame {frame}')
        app.key('Escape')
        app.wait_for(lambda: changed_fraction(idle, app.capture('settings-closed')) < 0.05, 'Settings did not close')

        h.step('clean exit')
        app.key('ctrl+shift+w')
        code = app.wait_exit(timeout=15)
        assert code == 0, f'App exited with {code}'
    print(f'PASS: menu and Settings colors remain stable through {6 * 3 + 5 * 3} hover redraws at {int(SCALE * 100)}% scale')
