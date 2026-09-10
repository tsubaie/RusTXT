"""Real keyboard/mouse and crash recovery checks. Run with xvfb-run -a."""
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1]).resolve()
output = Path(sys.argv[2] if len(sys.argv) > 2 else '/tmp/rustxt-iced-e2e')
output.mkdir(parents=True, exist_ok=True)

def wait_for(check, description):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (sqlite3.Error, FileNotFoundError, subprocess.CalledProcessError):
            pass
        time.sleep(0.1)
    raise AssertionError(description)

with tempfile.TemporaryDirectory(prefix='rustxt-iced-e2e-') as temp:
    root = Path(temp)
    env = dict(os.environ)
    for kind in ('CONFIG', 'DATA', 'CACHE', 'STATE'):
        env[f'XDG_{kind}_HOME'] = str(root / kind.lower())
    env['RUSTXT_DATA_DIR'] = str(root / 'data' / 'rustxt')
    env.pop('WAYLAND_DISPLAY', None)
    env['WINIT_UNIX_BACKEND'] = 'x11'
    database = root / 'data' / 'rustxt' / 'session.db'
    process = None
    log = (output / 'app.log').open('w')

    def rows():
        with sqlite3.connect(database) as db:
            return db.execute('SELECT id, content, dirty, is_open FROM documents ORDER BY tab_position').fetchall()

    def active_text():
        with sqlite3.connect(database) as db:
            return db.execute("SELECT content FROM documents WHERE id=(SELECT value FROM app_state WHERE key='active_id')").fetchone()[0]

    def xdo(*args):
        return subprocess.check_output(['xdotool', *args], env=env, text=True).strip()

    def launch(*args):
        global process
        process = subprocess.Popen([str(binary), *map(str, args)], env=env, stdout=log, stderr=log)
        window = wait_for(lambda: xdo('search', '--onlyvisible', '--pid', str(process.pid), '--name', '.'), 'Window did not appear').splitlines()[0]
        xdo('windowfocus', '--sync', window)
        time.sleep(0.5)
        return window

    def key(value):
        xdo('key', '--clearmodifiers', value)
        time.sleep(0.15)

    def type_text(value):
        for index, line in enumerate(value.split('\n')):
            if index:
                key('Return')
            xdo('type', '--clearmodifiers', '--delay', '15', line)

    try:
        window = launch()
        type_text('Recovery survives a crash.\nSecond line.')
        wait_for(lambda: active_text() == 'Recovery survives a crash.\nSecond line.', 'Typing not persisted')
        key('ctrl+z')
        wait_for(lambda: active_text() == 'Recovery survives a crash.\nSecond line', 'Undo failed')
        key('ctrl+shift+z')
        wait_for(lambda: active_text().endswith('line.'), 'Redo failed')
        process.kill()
        process.wait(timeout=5)
        window = launch()
        wait_for(lambda: active_text() == 'Recovery survives a crash.\nSecond line.', 'Crash restore failed')
        key('ctrl+z')
        wait_for(lambda: active_text().endswith('line'), 'Recovered undo history failed')
        key('ctrl+shift+z')
        wait_for(lambda: active_text().endswith('line.'), 'Recovered redo history failed')
        key('ctrl+t')
        type_text('A second unsaved note')
        wait_for(lambda: active_text() == 'A second unsaved note', 'Second tab failed')
        key('ctrl+w')
        wait_for(lambda: active_text().startswith('Recovery'), 'Close tab failed')
        key('ctrl+shift+t')
        wait_for(lambda: active_text() == 'A second unsaved note', 'Reopen failed')
        key('ctrl+h')
        type_text('second')
        key('Return')
        type_text('restored')
        wait_for(lambda: active_text() == 'A restored unsaved note', 'Find selection/replacement failed')
        key('Escape')
        sample = root / 'arabic.txt'
        sample.write_bytes('مرحبا بالعالم\r\nEnglish text 👋\r\n'.encode())
        subprocess.run([str(binary), str(sample)], env=env, check=True, timeout=10)
        wait_for(lambda: len([r for r in rows() if r[3]]) == 3, 'Single-instance forwarding failed')
        time.sleep(1)
        subprocess.run(['import', '-window', window, str(output / 'editor.png')], check=True, env=env)
        key('ctrl+End')
        type_text('end')
        key('ctrl+s')
        wait_for(lambda: sample.read_bytes().endswith(b'\r\nend'), 'Save did not preserve CRLF')
        sample.write_text('Changed by another program')
        type_text('!')
        key('ctrl+s')
        time.sleep(0.5)
        assert sample.read_text() == 'Changed by another program', 'External changes overwritten'
        subprocess.run(['import', '-window', window, str(output / 'conflict.png')], check=True, env=env)
        key('Escape')
        key('ctrl+comma')
        time.sleep(0.5)
        subprocess.run(['import', '-window', window, str(output / 'settings.png')], check=True, env=env)
        key('Escape')
        long_file = root / 'long.txt'
        long_file.write_text('A line of text for scrolling.\n' * 1000)
        subprocess.run([str(binary), str(long_file)], env=env, check=True, timeout=10)
        wait_for(lambda: len([r for r in rows() if r[3]]) == 4, 'Long file did not open')
        time.sleep(0.5)
        def scroll_line():
            import json
            with sqlite3.connect(database) as db:
                key_name = 'iced-history:' + db.execute("SELECT value FROM app_state WHERE key='active_id'").fetchone()[0]
                return json.loads(db.execute('SELECT value FROM app_state WHERE key=?', (key_name,)).fetchone()[0])['scroll'][0]
        xdo('mousemove', '--window', window, '400', '300')
        xdo('click', '--repeat', '12', '--delay', '60', '5')
        before_scroll = wait_for(lambda: scroll_line() if scroll_line() > 10 else None, 'Scroll position not saved')
        process.kill()
        process.wait(timeout=5)
        window = launch()
        xdo('mousemove', '--window', window, '400', '300')
        xdo('click', '5')
        wait_for(lambda: scroll_line() > before_scroll, 'Viewport did not restore after restart')
        key('ctrl+shift+w')
        process.wait(timeout=10)
        assert process.returncode == 0
        print('PASS: typing, undo/redo, crash recovery, tabs, reopen, find selection, file forwarding, CRLF save, external-change protection, settings, scroll recovery, clean exit')
    except Exception:
        if database.exists():
            print('Database at failure:', rows(), flush=True)
        subprocess.run(['import', '-window', 'root', str(output / 'failure.png')], env=env)
        raise
    finally:
        if process and process.poll() is None:
            process.kill()
            process.wait()
        log.close()
