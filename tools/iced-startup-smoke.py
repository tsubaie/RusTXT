"""Exercise native startup, file forwarding and recovery with isolated data."""
import os
import hashlib
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1])
if binary.is_dir():
    binary /= 'rustxt-iced.exe' if os.name == 'nt' else 'rustxt-iced'
binary = binary.resolve()
with tempfile.TemporaryDirectory(prefix='rustxt-startup-') as directory:
    root = Path(directory)
    env = dict(os.environ)
    for kind in ('CONFIG', 'DATA', 'CACHE', 'STATE'):
        env[f'XDG_{kind}_HOME'] = str(root / kind.lower())
    env['APPDATA'] = str(root / 'config')
    env['LOCALAPPDATA'] = str(root / 'local')
    env['RUSTXT_DATA_DIR'] = str(root / 'session')
    sample = root / 'مرحبا sample.txt'
    content = 'Native startup test\nمرحبا بالعالم\n'
    sample.write_bytes(content.encode('utf-8'))
    forwarded = root / 'forwarded.txt'
    forwarded.write_bytes(b'Second instance file\n')
    database = root / 'session/session.db'

    with (root / 'app.log').open('w+') as log:
        def launch(*paths):
            return subprocess.Popen([str(binary), *map(str, paths)], env=env,
                                    stdout=log, stderr=log)

        def stop(process):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()

        def wait_for_documents(process, expected):
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    log.seek(0)
                    raise AssertionError(f'Native app exited early ({process.returncode}): {log.read()}')
                if database.exists():
                    try:
                        with sqlite3.connect(database) as db:
                            actual = dict(db.execute('SELECT file_path, disk_fingerprint FROM documents WHERE is_open = 1'))
                        if all(actual.get(str(path)) == hashlib.sha256(text.encode('utf-8')).hexdigest() for path, text in expected.items()):
                            return
                    except sqlite3.OperationalError:
                        pass
                time.sleep(0.1)
            log.seek(0)
            raise AssertionError(f'Native app did not recover expected file contents: {log.read()}')

        process = launch(sample)
        try:
            wait_for_documents(process, {sample: content})
            second = launch(forwarded)
            try:
                assert second.wait(timeout=15) == 0, 'Second instance failed to forward its file'
            finally:
                stop(second)
            expected = {sample: content, forwarded: 'Second instance file\n'}
            wait_for_documents(process, expected)
            stop(process)
            process = launch()
            wait_for_documents(process, expected)
            time.sleep(3)
            assert process.poll() is None, 'Native app exited after recovery'
            print('PASS: native startup, Unicode file loading, single-instance forwarding and session recovery')
        finally:
            stop(process)
