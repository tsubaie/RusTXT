"""Launch a native build with an isolated recovery database and a UTF-8 file."""
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1]) / ('rustxt-iced.exe' if os.name == 'nt' else 'rustxt-iced')
binary = binary.resolve()
with tempfile.TemporaryDirectory(prefix='rustxt-startup-') as directory:
    root = Path(directory)
    env = dict(os.environ)
    for kind in ('CONFIG', 'DATA', 'CACHE', 'STATE'):
        env[f'XDG_{kind}_HOME'] = str(root / kind.lower())
    env['APPDATA'] = str(root / 'config')
    env['LOCALAPPDATA'] = str(root / 'local')
    env['RUSTXT_DATA_DIR'] = str(root / 'session')
    sample = root / 'sample.txt'
    content = 'Native startup test\nمرحبا بالعالم\n'
    sample.write_text(content, encoding='utf-8')
    with (root / 'app.log').open('w+') as log:
        process = subprocess.Popen([str(binary), str(sample)], env=env, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 20
            recovered = False
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    log.seek(0)
                    raise AssertionError(f'Native app exited early ({process.returncode}): {log.read()}')
                database = root / 'session/session.db'
                if database.exists():
                    try:
                        with sqlite3.connect(database) as db:
                            recovered = db.execute('SELECT COUNT(*) FROM documents WHERE is_open = 1').fetchone()[0] > 0
                    except sqlite3.OperationalError:
                        pass
                if recovered:
                    break
                time.sleep(0.1)
            assert recovered, 'Native app did not initialize its recovery session'
            time.sleep(3)
            assert process.poll() is None, 'Native app exited after initialization'
            print('PASS: native app starts, opens a recovery session and remains running')
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
