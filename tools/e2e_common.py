"""Shared helpers for the Iced end-to-end harness (tools/iced-*.py).

Every synchronisation point polls an observable (an X11 window, a row in the
session database, pixels in a screenshot) instead of sleeping for a fixed
time, so the scripts pass on slow CI runners and fail fast with a reason when
the app misbehaves. The only fixed delays are input pacing (KEY_PACING) and a
named render settle (RENDER_SETTLE) used right before a screenshot is compared.

The module is imported by iced-startup-smoke.py on Windows and macOS too, so
X11/ImageMagick helpers only call their tools when used.
"""
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import time
import traceback
from contextlib import closing
from pathlib import Path

DEFAULT_TIMEOUT = 20.0
"""Seconds a wait_for() predicate gets before the step fails."""
WINDOW_TIMEOUT = 30.0
"""Seconds the app gets to map its window after launch (cold font caches)."""
POLL_INTERVAL = 0.1
KEY_PACING = 0.15
"""Seconds between injected key chords. This is input pacing, not
synchronisation: winit coalesces modifier changes that arrive in the same
event batch, so ctrl+s immediately followed by ctrl+comma can lose a modifier.
State changes are always waited for separately."""
RENDER_SETTLE = 0.25
"""Seconds between the last injected event and a screenshot that is compared
against a reference. Rendering happens on the next frame after the update
loop persisted state, so a settle after observing the database is needed."""
TRANSIENT_ERRORS = (
    sqlite3.Error,
    FileNotFoundError,
    subprocess.CalledProcessError,
    subprocess.TimeoutExpired,
    IndexError,
    TypeError,
    ValueError,
)

IMAGEMAGICK = shutil.which('magick') or 'convert'


def sha256(data):
    if isinstance(data, str):
        data = data.encode('utf-8')
    return hashlib.sha256(data).hexdigest()


def wait_for(check, description, timeout=DEFAULT_TIMEOUT, interval=POLL_INTERVAL, process=None, log=None):
    """Poll check() until it returns a truthy value, which is returned.

    Transient errors (database not created yet, window not mapped, ...) are
    retried. If `process` exits meanwhile the wait fails immediately with the
    tail of `log`, so a crash is reported as a crash and not as a timeout.
    """
    deadline = time.monotonic() + timeout
    last_error = None
    while True:
        if process is not None and process.poll() is not None:
            raise AssertionError(f'{description}: app exited early with code {process.returncode}{log_tail(log)}')
        try:
            value = check()
            if value:
                return value
        except TRANSIENT_ERRORS as error:
            last_error = error
        if time.monotonic() >= deadline:
            detail = f' (last error: {last_error!r})' if last_error else ''
            raise AssertionError(f'{description}: not observed within {timeout:.0f}s{detail}')
        time.sleep(interval)


def wait_stable(process, seconds, description, log=None):
    """Fail as soon as `process` exits during a bounded stability window.

    Liveness after recovery can only be observed over time; polling instead
    of sleeping reports an early exit immediately with its log.
    """
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(f'{description}: app exited with code {process.returncode}{log_tail(log)}')
        time.sleep(POLL_INTERVAL)


def log_tail(log, lines=40):
    """The last lines of an app log file (path or open file), for messages."""
    if log is None:
        return ''
    try:
        path = Path(getattr(log, 'name', log))
        text = path.read_text(errors='replace').splitlines()[-lines:]
    except OSError:
        return ''
    return ('\n--- app log ---\n' + '\n'.join(text)) if text else ''


def sandbox_env(root, data_dir='data/rustxt', scale=None):
    """Environment isolating config, data and session state under `root`."""
    root = Path(root)
    env = dict(os.environ)
    for kind in ('CONFIG', 'DATA', 'CACHE', 'STATE'):
        env[f'XDG_{kind}_HOME'] = str(root / kind.lower())
    env['APPDATA'] = str(root / 'config')
    env['LOCALAPPDATA'] = str(root / 'local')
    env['RUSTXT_DATA_DIR'] = str(root / data_dir)
    env.pop('WAYLAND_DISPLAY', None)
    env['WINIT_UNIX_BACKEND'] = 'x11'
    if scale is not None:
        env['WINIT_X11_SCALE_FACTOR'] = str(scale)
    return env


def resolve_binary(argument):
    binary = Path(argument)
    if binary.is_dir():
        binary /= 'rustxt-iced.exe' if os.name == 'nt' else 'rustxt-iced'
    return binary.resolve()


class Database:
    """Read-only view of the session database the app writes every 250 ms."""

    def __init__(self, path):
        self.path = Path(path)

    def query(self, sql, *params):
        # file:///... URI (as_uri() handles Windows drive letters); read-only
        # so a poll never creates an empty database before the app does.
        with closing(sqlite3.connect(self.path.absolute().as_uri() + '?mode=ro', uri=True)) as db:
            return db.execute(sql, params).fetchall()

    def rows(self):
        return self.query('SELECT id, file_path, content, dirty, is_open, disk_fingerprint FROM documents ORDER BY tab_position')

    def open_rows(self):
        return [row for row in self.rows() if row[4]]

    def document(self, file_path):
        """(content, dirty, disk_fingerprint) of the open document at `file_path`."""
        rows = self.query('SELECT content, dirty, disk_fingerprint FROM documents WHERE is_open = 1 AND file_path = ?', str(file_path))
        return rows[0] if rows else None

    def state(self, key):
        rows = self.query('SELECT value FROM app_state WHERE key = ?', key)
        return rows[0][0] if rows else None

    def active_id(self):
        return self.state('active_id')

    def active_text(self):
        rows = self.query("SELECT content FROM documents WHERE id = (SELECT value FROM app_state WHERE key = 'active_id')")
        return rows[0][0]

    def active_file(self):
        rows = self.query("SELECT file_path FROM documents WHERE id = (SELECT value FROM app_state WHERE key = 'active_id')")
        return rows[0][0]

    def active_scroll_line(self):
        """First visible line of the active editor, from the Iced view state."""
        return json.loads(self.state('iced-history:' + self.active_id()))['scroll'][0]

    def open_fingerprints(self):
        return dict(self.query('SELECT file_path, disk_fingerprint FROM documents WHERE is_open = 1'))

    def dump(self):
        try:
            return {'documents': self.rows(), 'app_state': self.query('SELECT key, value FROM app_state')}
        except TRANSIENT_ERRORS as error:
            return {'error': repr(error)}


def xdo(env, *args, timeout=30):
    """Run xdotool; the timeout kills a hung child (e.g. search --sync)."""
    return subprocess.run(['xdotool', *map(str, args)], env=env, text=True, capture_output=True, timeout=timeout, check=True).stdout.strip()


def window_geometry(env, window):
    """{'X','Y','WIDTH','HEIGHT','SCREEN'} of a window in physical pixels."""
    values = {}
    for line in xdo(env, 'getwindowgeometry', '--shell', window).splitlines():
        key, _, value = line.partition('=')
        values[key] = int(value)
    return values


class App:
    """Launch, drive and stop one rustxt-iced instance under X11."""

    def __init__(self, binary, env, log, output):
        self.binary = Path(binary)
        self.env = env
        self.log = log
        self.output = Path(output)
        self.process = None
        self.window = None

    @property
    def alive(self):
        return self.process is not None and self.process.poll() is None

    def run_once(self, *args, timeout=15):
        """A second instance: forwards its arguments and exits 0."""
        result = subprocess.run([str(self.binary), *map(str, args)], env=self.env, stdout=self.log, stderr=self.log, timeout=timeout)
        assert result.returncode == 0, f'Second instance exited with {result.returncode}{log_tail(self.log)}'

    def launch(self, *args, timeout=WINDOW_TIMEOUT):
        """Start the app and return its focused window id."""
        self.process = subprocess.Popen([str(self.binary), *map(str, args)], env=self.env, stdout=self.log, stderr=self.log)
        self.window = self.wait_window(timeout)
        self.focus()
        return self.window

    def wait_window(self, timeout=WINDOW_TIMEOUT):
        """Block until the app maps a visible window (xdotool search --sync)."""
        deadline = time.monotonic() + timeout
        while True:
            if self.process.poll() is not None:
                raise AssertionError(f'App exited with code {self.process.returncode} before mapping a window{log_tail(self.log)}')
            try:
                found = xdo(self.env, 'search', '--sync', '--onlyvisible', '--pid', self.process.pid, '--name', '.', timeout=2)
            except (subprocess.TimeoutExpired, subprocess.CalledProcessError):
                found = ''
            if found.split():
                return found.split()[0]
            if time.monotonic() >= deadline:
                raise AssertionError(f'Window did not appear within {timeout:.0f}s{log_tail(self.log)}')

    def focus(self):
        xdo(self.env, 'windowfocus', '--sync', self.window)
        wait_for(lambda: xdo(self.env, 'getwindowfocus') == self.window, 'Window did not take keyboard focus', process=self.process, log=self.log)

    def geometry(self):
        return window_geometry(self.env, self.window)

    def resize(self, width, height):
        """Resize and wait until the X server reports the new size."""
        xdo(self.env, 'windowsize', self.window, width, height)
        wait_for(lambda: (self.geometry()['WIDTH'], self.geometry()['HEIGHT']) == (width, height), f'Window did not resize to {width}x{height}', process=self.process, log=self.log)

    def key(self, chord):
        xdo(self.env, 'key', '--clearmodifiers', chord)
        time.sleep(KEY_PACING)

    def type_text(self, value):
        for index, line in enumerate(value.split('\n')):
            if index:
                self.key('Return')
            xdo(self.env, 'type', '--clearmodifiers', '--delay', '15', line)

    def mouse_move(self, x, y):
        xdo(self.env, 'mousemove', '--window', self.window, round(x), round(y))

    def click(self, button, repeat=1, delay=60):
        xdo(self.env, 'click', '--repeat', repeat, '--delay', delay, button)

    def capture(self, name):
        """Screenshot the app window into the output dir; returns the path."""
        name = str(name)
        path = Path(name) if name.endswith('.png') else self.output / f'{name}.png'
        subprocess.run(['import', '-window', self.window, str(path)], env=self.env, check=True, timeout=30, stderr=subprocess.DEVNULL)
        return path

    def wait_for(self, check, description, timeout=DEFAULT_TIMEOUT):
        return wait_for(check, description, timeout=timeout, process=self.process, log=self.log)

    def wait_exit(self, timeout=15):
        self.process.wait(timeout=timeout)
        return self.process.returncode

    def kill(self):
        """SIGKILL: simulates a crash, nothing gets a chance to flush."""
        if self.alive:
            self.process.kill()
        if self.process is not None:
            self.process.wait(timeout=10)

    def stop(self):
        if self.alive:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()


class Image:
    """8-bit RGB raster decoded through ImageMagick (PPM, header carries size)."""

    def __init__(self, width, height, data):
        self.width, self.height, self.data = width, height, data

    @classmethod
    def load(cls, path, crop=None):
        raw = subprocess.run([IMAGEMAGICK, str(path), *_crop_args(crop), '-depth', '8', 'ppm:-'],
                             capture_output=True, check=True, timeout=30).stdout
        width, height, offset = _ppm_header(raw, b'P6')
        data = raw[offset:offset + width * height * 3]
        assert len(data) == width * height * 3, 'Truncated PPM data'
        return cls(width, height, data)

    def pixel(self, x, y):
        offset = (y * self.width + x) * 3
        return tuple(self.data[offset:offset + 3])


def _ppm_header(raw, magic):
    assert raw.startswith(magic), f'ImageMagick did not produce {magic!r}'
    fields = []
    offset = 2
    while len(fields) < 3:
        while raw[offset:offset + 1].isspace():
            offset += 1
        if raw[offset:offset + 1] == b'#':
            offset = raw.index(b'\n', offset) + 1
            continue
        end = offset
        while not raw[end:end + 1].isspace():
            end += 1
        fields.append(int(raw[offset:end]))
        offset = end
    return fields[0], fields[1], offset + 1


def _crop_args(crop):
    if not crop:
        return []
    x, y, w, h = crop
    return ['-crop', f'{w}x{h}+{x}+{y}', '+repage']


class InkMask:
    """1 byte per pixel: 0xff where a screenshot region differs from its
    background colour by more than `fuzz` (ImageMagick colour distance,
    15% by default: anti-aliased glyph edges count, 1-unit noise does not)."""

    def __init__(self, width, height, data):
        self.width, self.height, self.data = width, height, data

    @classmethod
    def load(cls, path, background, crop=None, fuzz='15%'):
        color = 'srgb(%d,%d,%d)' % tuple(background)
        raw = subprocess.run(
            [IMAGEMAGICK, str(path), *_crop_args(crop), '-fuzz', fuzz, '-fill', 'black', '-opaque', color,
             '-fill', 'white', '+opaque', 'black', '-depth', '8', 'pgm:-'],
            capture_output=True, check=True, timeout=30).stdout
        width, height, offset = _ppm_header(raw, b'P5')
        data = raw[offset:offset + width * height]
        assert len(data) == width * height, 'Truncated PGM data'
        return cls(width, height, data)

    def row(self, y):
        return self.data[y * self.width:(y + 1) * self.width]

    def count(self):
        return self.data.count(b'\xff')

    def ink_bands(self):
        """Runs of consecutive rows containing ink, as (top, bottom) pairs:
        one band per rendered text line in an editor crop."""
        bands = []
        top = None
        for y in range(self.height):
            inked = b'\xff' in self.row(y)
            if inked and top is None:
                top = y
            elif not inked and top is not None:
                bands.append((top, y - 1))
                top = None
        if top is not None:
            bands.append((top, self.height - 1))
        return bands


def changed_fraction(path_a, path_b, threshold='3%'):
    """Fraction of pixels differing between two same-size screenshots on any
    channel by more than `threshold` (3% of full scale, about 8 units, absorbs
    blending noise). Computed by ImageMagick: per-channel difference, max over
    channels, threshold, mean of the resulting mask."""
    result = subprocess.run(
        [IMAGEMAGICK, str(path_a), str(path_b), '-compose', 'difference', '-composite', '-separate',
         '-evaluate-sequence', 'Max', '-threshold', threshold, '-format', '%[fx:mean]', 'info:'],
        capture_output=True, text=True, check=True, timeout=60)
    return float(result.stdout.strip())


def color_close(actual, expected, tolerance=2):
    """Per-channel tolerance: Xvfb's 24-bit visual, wgpu's sRGB conversion and
    tiny-skia's blending each round differently, so two correct frames may
    differ by one unit per channel. A hover regression tints a panel by tens
    of units, which this still catches."""
    return all(abs(a - e) <= tolerance for a, e in zip(actual, expected))


class Harness:
    """Output dir, app log and failure artifacts for one e2e script.

    with Harness(binary, output, root, env) as h:
        h.app.launch()
        ...
    On any exception the window (or root screen) screenshot, the database
    dump and the traceback are written to the output dir before re-raising;
    the app is always killed on the way out.
    """

    def __init__(self, binary, output, root, env, database=None):
        self.output = Path(output)
        self.output.mkdir(parents=True, exist_ok=True)
        self.root = Path(root)
        self.env = env
        self.log = (self.output / 'app.log').open('w')
        self.app = App(binary, env, self.log, self.output)
        self.database = Database(database or self.root / 'data' / 'rustxt' / 'session.db')
        self.steps = (self.output / 'steps.log').open('w')

    def step(self, name):
        self.steps.write(f'{time.strftime("%H:%M:%S")} {name}\n')
        self.steps.flush()
        print(f'  step: {name}', flush=True)

    def __enter__(self):
        return self

    def __exit__(self, kind, error, tb):
        try:
            if error is not None:
                self.steps.write(''.join(traceback.format_exception(kind, error, tb)))
                self.record_failure()
        finally:
            try:
                self.app.kill()
            finally:
                self.log.close()
                self.steps.close()
        return False

    def record_failure(self):
        target = self.app.window if self.app.alive and self.app.window else 'root'
        subprocess.run(['import', '-window', target, str(self.output / 'failure.png')], env=self.env, timeout=30, stderr=subprocess.DEVNULL)
        if target != 'root':
            subprocess.run(['import', '-window', 'root', str(self.output / 'failure-screen.png')], env=self.env, timeout=30, stderr=subprocess.DEVNULL)
        dump = self.database.dump() if self.database.path.exists() else {'error': 'no database'}
        (self.output / 'database-at-failure.json').write_text(json.dumps(dump, indent=1, default=str))
        print('Documents at failure:', dump.get('documents', dump), flush=True)
        print(f'Full database dump: {self.output / "database-at-failure.json"}', flush=True)
        self.log.flush()
        print(log_tail(self.log), flush=True)
