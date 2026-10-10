#!/usr/bin/env python3
"""Drive the daemon's tray icon over a private D-Bus session (PRD §8.1)."""

import argparse
import array
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("smoke", Path(__file__).with_name("pipewire-smoke.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


def png(width, height, argb):
    """Encode ARGB32 (network order) pixels as PNG."""
    rows = b"".join(b"\0" + b"".join(bytes(argb[i + 1:i + 4]) + bytes(argb[i:i + 1])
                                     for i in range((y * width) * 4, (y + 1) * width * 4, 4))
                    for y in range(height))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=Path, default=ROOT / "target/release/macafond")
    parser.add_argument("--ctl", type=Path, default=ROOT / "target/release/macafon")
    parser.add_argument("--icons", type=Path, help="save the 48 px icon of each state as PNG here")
    parser.add_argument("--private-session", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    for command in ("dbus-run-session", "gdbus", "pipewire", "pw-cat"):
        if not shutil.which(command):
            parser.error(f"{command} missing")
    if not args.daemon.is_file() or not args.ctl.is_file():
        parser.error("build first: cargo build --release -p macafond -p macafon-cli")
    if not args.private_session:
        extra = ["--icons", str(args.icons.resolve())] if args.icons else []
        return subprocess.call(["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                                "--private-session", "--daemon", str(args.daemon.resolve()),
                                "--ctl", str(args.ctl.resolve()), *extra])
    processes, logs = [], []
    with tempfile.TemporaryDirectory(prefix="macafon-tray-") as temporary:
        directory = Path(temporary)
        runtime = directory / "runtime"
        runtime.mkdir(mode=0o700)
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), PIPEWIRE_RUNTIME_DIR=str(runtime),
                   XDG_CONFIG_HOME=str(directory / "config"), XDG_STATE_HOME=str(directory / "state"))
        config = directory / "test.conf"
        config.write_text(smoke.CONFIG)

        def spawn(command):
            log = (directory / f"process-{len(logs)}.log").open("w+")
            logs.append(log)
            process = subprocess.Popen([str(arg) for arg in command], env=env, stdout=log, stderr=log)
            processes.append(process)
            return process

        def run(*command):
            return subprocess.check_output([str(arg) for arg in command], env=env, text=True,
                                           stderr=subprocess.PIPE, timeout=8)

        def status():
            return json.loads(run(args.ctl, "ctl", "status", "--json"))

        def tray_name():
            names = run("gdbus", "call", "--session", "--dest", "org.freedesktop.DBus",
                        "--object-path", "/org/freedesktop/DBus", "--method",
                        "org.freedesktop.DBus.ListNames")
            return next(iter(re.findall(r"'(org\.kde\.StatusNotifierItem-[^']+)'", names)), None)

        def call(name, path, method, *arguments):
            return run("gdbus", "call", "--session", "--dest", name, "--object-path", path,
                       "--method", method, *arguments)

        def prop(name, key):
            return call(name, "/StatusNotifierItem", "org.freedesktop.DBus.Properties.Get",
                        "org.kde.StatusNotifierItem", key)

        def icon(name):
            """The largest pixmap as (width, height, ARGB bytes)."""
            text = prop(name, "IconPixmap")
            icons = re.findall(r"\((\d+), (\d+), (?:\[byte )?\[?([^\]]*)\]", text)
            width, height, data = max(icons, key=lambda found: int(found[0]))
            return int(width), int(height), [int(value, 16) for value in re.findall(r"0x([0-9a-f]+)", data)]

        def menu_ids(name):
            layout = call(name, "/MenuBar", "com.canonical.dbusmenu.GetLayout", "0", "10", "['label']")
            return {label: int(item) for item, label in
                    re.findall(r"\((\d+), \{[^}]*'label': <'([^']*)'>", layout)}

        def settled_icon(name, rgb):
            """Wait for the transition's final colour before comparing or saving."""
            def ready():
                result = icon(name)
                data = result[2]
                pixels = [data[i:i + 4] for i in range(0, len(data), 4) if data[i]]
                return result if pixels and all(pixel[1:] == rgb for pixel in pixels) else None
            return smoke.wait_for(ready, "settled tray animation")

        def click(name, label):
            ids = menu_ids(name)
            assert label in ids, f"{label!r} not in tray menu {sorted(ids)}"
            call(name, "/MenuBar", "com.canonical.dbusmenu.Event", str(ids[label]), "clicked",
                 "<int32 0>", "0")

        def save_icon(name, state):
            if args.icons:
                width, height, data = icon(name)
                args.icons.mkdir(parents=True, exist_ok=True)
                (args.icons / f"tray-{state}.png").write_bytes(png(width, height, data))

        try:
            spawn(["pipewire", "-c", config])
            smoke.wait_for(lambda: (runtime / "pipewire-0").exists(), "private PipeWire server")
            tone = array.array("f", [0.1 * math.sin(2 * math.pi * 440 * n / 48000) for n in range(48000 * 30)])
            (directory / "tone.raw").write_bytes(tone.tobytes())
            spawn(["pw-cat", "--playback", "--raw", "--format", "f32", "--channels", "1",
                   "--channel-map", "MONO", "--target", "0", "--latency", "128", "--properties",
                   "node.name=test_microphone media.class=Audio/Source adapter.auto-port-config={ mode=dsp position=preserve }",
                   directory / "tone.raw"])
            daemon = spawn([args.daemon, "run", "--input", "test_microphone"])
            name = smoke.wait_for(tray_name, "tray D-Bus name")
            smoke.wait_for(lambda: status()["input_connected"], "microphone linked")
            smoke.wait_for(lambda: "Preset: Natural" in prop(name, "ToolTip")
                           and "disconnected" not in prop(name, "ToolTip"), "tooltip")
            assert "'Hardware'" in prop(name, "Category")
            normal = settled_icon(name, [255, 255, 255])
            assert normal[0] == 48 and any(normal[2][0::4]), "empty icon"
            save_icon(name, "normal")

            click(name, "Mute microphone")
            smoke.wait_for(lambda: status()["settings"]["muted"], "mute from the tray")
            smoke.wait_for(lambda: "Microphone muted" in prop(name, "ToolTip"), "muted tooltip")
            muted = settled_icon(name, [255, 98, 89])
            assert muted != normal, "muted icon looks normal"
            save_icon(name, "muted")
            call(name, "/StatusNotifierItem", "org.kde.StatusNotifierItem.SecondaryActivate", "0", "0")
            smoke.wait_for(lambda: not status()["settings"]["muted"], "middle click unmutes")
            assert settled_icon(name, [255, 255, 255]) == normal, "unmute did not restore the upright logo"

            click(name, "Bypass processing")
            smoke.wait_for(lambda: status()["settings"]["bypass"], "bypass from the tray")
            bypass = settled_icon(name, [255, 98, 89])
            assert bypass != muted, "bypass needs its own pause badge"
            save_icon(name, "bypass")
            click(name, "Bypass processing")

            click(name, "Broadcast")
            smoke.wait_for(lambda: status()["preset"] == "broadcast", "preset from the tray")
            run(args.ctl, "ctl", "presets", "save", "Tray test")
            smoke.wait_for(lambda: "Tray test" in menu_ids(name), "user preset in the tray menu")

            run(args.ctl, "ctl", "device", "missing_microphone")
            smoke.wait_for(lambda: "disconnected" in prop(name, "ToolTip"), "disconnected tooltip")
            assert settled_icon(name, [255, 98, 89]) != muted, "error needs its own dot"
            save_icon(name, "disconnected")
            print("tray: tooltip, icon states, mute/bypass/preset menu, middle-click mute, "
                  "user presets listed")

            try:
                click(name, "Quit Macafon")
            except subprocess.CalledProcessError as error:
                # Quit can close SNI before dbusmenu sends its reply. Only accept
                # that race; the clean exit and socket cleanup remain mandatory.
                if "org.freedesktop.DBus.Error.NoReply" not in error.stderr:
                    raise
            assert daemon.wait(timeout=5) == 0, f"daemon exit code {daemon.returncode}"
            assert not (runtime / "macafon" / "macafond.sock").exists(), "socket left behind"
            print("tray: Quit stops the daemon cleanly")
        except Exception as error:
            if isinstance(error, subprocess.CalledProcessError):
                print(error.stderr, file=sys.stderr)
            for log in logs:
                log.flush()
                log.seek(0)
                print(log.read(), file=sys.stderr)
            raise
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            for log in logs:
                log.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
