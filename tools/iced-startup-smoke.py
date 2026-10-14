"""Exercise native startup, file forwarding and recovery with isolated data.

    python tools/iced-startup-smoke.py target/release        # directory or binary

Runs without X11 tooling (natively on Windows/macOS, under xvfb-run for the
installed Linux packages). The oracle is the session database: a document is
"loaded" when its row's disk_fingerprint equals the SHA-256 of the bytes we
wrote, and recovery after SIGKILL must reproduce the same rows.
"""
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2e_common import Database, log_tail, resolve_binary, sandbox_env, sha256, wait_for, wait_stable  # noqa: E402

binary = resolve_binary(sys.argv[1])
LOAD_TIMEOUT = 30
"""Seconds for a cold start to load fonts and write the first snapshot."""
STABILITY_WINDOW = 3
"""Seconds the recovered instance must keep running; polled, so an early
exit is reported immediately with its log."""

with tempfile.TemporaryDirectory(prefix='rustxt-startup-') as directory:
    root = Path(directory)
    env = sandbox_env(root, data_dir='session')
    sample = root / 'مرحبا sample.txt'
    content = 'Native startup test\nمرحبا بالعالم\n'
    sample.write_bytes(content.encode('utf-8'))
    forwarded = root / 'forwarded.txt'
    forwarded.write_bytes(b'Second instance file\n')
    db = Database(root / 'session/session.db')

    with (root / 'app.log').open('w+') as log:
        def launch(*paths):
            return subprocess.Popen([str(binary), *map(str, paths)], env=env, stdout=log, stderr=log)

        def stop(process):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()

        def crash(process):
            """SIGKILL (TerminateProcess on Windows): no shutdown hook runs."""
            process.kill()
            process.wait(timeout=10)

        def wait_for_documents(process, expected):
            """Until every expected path is open with the fingerprint of its bytes."""
            wanted = {str(path): sha256(text) for path, text in expected.items()}
            wait_for(lambda: all(db.open_fingerprints().get(path) == digest for path, digest in wanted.items()),
                     f'Native app did not load {sorted(wanted)}', timeout=LOAD_TIMEOUT, process=process, log=log)

        process = launch(sample)
        try:
            wait_for_documents(process, {sample: content})
            second = launch(forwarded)
            try:
                assert second.wait(timeout=15) == 0, f'Second instance failed to forward its file{log_tail(log)}'
            finally:
                stop(second)
            expected = {sample: content, forwarded: 'Second instance file\n'}
            wait_for_documents(process, expected)
            crash(process)
            process = launch()
            wait_for_documents(process, expected)
            wait_stable(process, STABILITY_WINDOW, 'Native app exited after recovery', log=log)
            print('PASS: native startup, Unicode file loading, single-instance forwarding and crash recovery')
        except Exception:
            log.flush()
            print(log_tail(log), file=sys.stderr, flush=True)
            raise
        finally:
            stop(process)
