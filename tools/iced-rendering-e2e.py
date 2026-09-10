"""Check panel colors during repeated hover redraws at fractional display scale.

Run: xvfb-run -a -s '-screen 0 1600x1200x24' python3 tools/iced-rendering-e2e.py BINARY OUTPUT
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1]).resolve()
output = Path(sys.argv[2])
output.mkdir(parents=True, exist_ok=True)
scale = 1.25
with tempfile.TemporaryDirectory(prefix='rustxt-rendering-') as temp:
    root = Path(temp)
    env = dict(os.environ)
    for kind in ('CONFIG', 'DATA', 'CACHE', 'STATE'):
        env[f'XDG_{kind}_HOME'] = str(root / kind.lower())
    env.update(RUSTXT_DATA_DIR=str(root / 'data/rustxt'), WINIT_X11_SCALE_FACTOR=str(scale))
    env.pop('WAYLAND_DISPLAY', None)
    config = root / 'config/rustxt'
    (config / 'themes').mkdir(parents=True)
    (config / 'config.toml').write_text('[appearance]\ntheme="parity"\n[window]\ntitle_bar="hide"\n')
    (config / 'themes/parity.toml').write_text('mode="dark"\nbackground="#1e1e2e"\nforeground="#cdd6f4"\nchrome="#161622"\nmenu="#313244"\naccent="#89b4fa"\n')

    def xdo(*args):
        return subprocess.check_output(['xdotool', *args], env=env, text=True).strip()

    def hover(x, y):
        xdo('mousemove', '--window', window, str(round(x * scale)), str(round(y * scale)))
        time.sleep(0.12)

    def capture(name):
        subprocess.run(['import', '-window', window, str(output / f'{name}.png')], env=env, check=True)

    def pixel(x, y):
        raw = subprocess.check_output(['import', '-window', window, '-depth', '8', 'rgb:-'], env=env)
        offset = (round(y * scale) * 1250 + round(x * scale)) * 3
        return tuple(raw[offset:offset + 3])

    with (output / 'app.log').open('w') as log:
        process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)
        try:
            time.sleep(2)
            window = xdo('search', '--onlyvisible', '--pid', str(process.pid), '--name', '.').splitlines()[0]
            xdo('windowsize', window, '1250', '875')
            xdo('windowfocus', '--sync', window)
            time.sleep(0.5)
            xdo('key', '--clearmodifiers', 'alt+e')
            time.sleep(0.3)
            capture('menu-before')
            for frame, y in enumerate([242, 319, 351, 415, 492, 242] * 3):
                hover(120, y)
                value = pixel(60, 89)
                assert value == (49, 50, 68), f'Menu changed color at frame {frame}: {value}'
            capture('menu-after')
            xdo('key', '--clearmodifiers', 'Escape')
            xdo('key', '--clearmodifiers', 'ctrl+comma')
            time.sleep(0.4)
            capture('settings-before')
            for frame, (x, y) in enumerate([(730, 170), (730, 275), (700, 360), (750, 450), (700, 540)] * 3):
                hover(x, y)
                value = pixel(190, 210)
                assert value == (22, 22, 34), f'Settings changed color at frame {frame}: {value}'
            capture('settings-after')
            xdo('key', '--clearmodifiers', 'Escape')
            xdo('key', '--clearmodifiers', 'ctrl+shift+w')
            process.wait(timeout=10)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
print('PASS: menu and Settings colors remain stable through 33 hover redraws at 125% scale')
