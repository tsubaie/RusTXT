"""Run under xvfb-run. Measures fresh isolated sessions; never opens user data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument("binary", type=Path)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
binary = args.binary.resolve()
results = {"binary": str(binary), "binary_bytes": binary.stat().st_size, "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "runs": []}
for workload in ("empty", "1mib"):
    for repeat in range(3):
        with tempfile.TemporaryDirectory(prefix="rustxt-footprint-") as root:
            root = Path(root)
            env = dict(os.environ)
            for kind in ("CONFIG", "DATA", "CACHE", "STATE"):
                env[f"XDG_{kind}_HOME"] = str(root / kind.lower())
            env.update(GDK_BACKEND="x11", GSK_RENDERER="cairo", WINIT_UNIX_BACKEND="x11")
            env.pop("WAYLAND_DISPLAY", None)
            env["RUSTXT_DATA_DIR"] = str(root / "data" / "rustxt")
            command = [str(binary)]
            if workload == "1mib":
                sample = root / "sample.txt"
                line = "A plain text note with English and Arabic: مرحبا بالعالم.\n"
                sample.write_text((line * (1048576 // len(line.encode()) + 1)), encoding="utf-8")
                command.append(str(sample))
            with (root / "app.log").open("w+") as log:
                process = subprocess.Popen(command, env=env, stdout=log, stderr=log)
                try:
                    deadline = time.monotonic() + 15
                    window = None
                    while time.monotonic() < deadline:
                        found = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(process.pid), "--name", "."], env=env, capture_output=True, text=True)
                        if found.returncode == 0 and found.stdout.strip():
                            window = found.stdout.splitlines()[0]
                            break
                        if process.poll() is not None:
                            log.seek(0)
                            raise RuntimeError(log.read())
                        time.sleep(0.1)
                    if window is None:
                        raise RuntimeError("No visible application window")
                    subprocess.run(["xdotool", "windowsize", window, "1000", "700"], env=env, check=True)
                    time.sleep(5)
                    samples = []
                    for _ in range(5):
                        if process.poll() is not None:
                            log.seek(0)
                            raise RuntimeError(log.read())
                        rollup = Path(f"/proc/{process.pid}/smaps_rollup").read_text()
                        values = {}
                        for line in rollup.splitlines():
                            fields = line.split()
                            if fields[0] in ("Rss:", "Pss:", "Private_Clean:", "Private_Dirty:"):
                                values[fields[0][:-1]] = int(fields[1])
                        values["USS"] = values["Private_Clean"] + values["Private_Dirty"]
                        samples.append(values)
                        time.sleep(0.2)
                    results["runs"].append({"workload": workload, "repeat": repeat,
                        **{name + "_kib": statistics.median(s[name] for s in samples)
                           for name in ("Rss", "Pss", "USS")}})
                finally:
                    process.terminate()
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
results["method"] = "Xvfb 1280x1024, window 1000x700, software rendering, 5s warmup, median of 5 samples, 3 fresh sessions per workload"
args.output.write_text(json.dumps(results, indent=2) + "\n")
print(json.dumps(results, indent=2))
