#!/usr/bin/env python3
"""Global hotkeys through a fake GlobalShortcuts portal in a private D-Bus session (§8.3).

Checks binding with the preferred trigger, conflicts inside Macafon (HK-002), toggle and hold
actions (HK-001), preset and channel actions with OSD notifications (HK-006), CLI commands.
"""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("smoke", Path(__file__).with_name("pipewire-smoke.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=Path, default=ROOT / "target/release/macafond")
    parser.add_argument("--ctl", type=Path, default=ROOT / "target/release/macafon")
    parser.add_argument("--private-session", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    for command in ("dbus-run-session", "pipewire", "gdbus"):
        if not shutil.which(command):
            parser.error(f"{command} missing")
    if not args.private_session:
        return subprocess.call(["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                                "--private-session", "--daemon", str(args.daemon.resolve()),
                                "--ctl", str(args.ctl.resolve())])
    processes = []
    with tempfile.TemporaryDirectory(prefix="macafon-hotkeys-") as temporary:
        directory = Path(temporary)
        runtime = directory / "runtime"
        runtime.mkdir(mode=0o700)
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), PIPEWIRE_RUNTIME_DIR=str(runtime),
                   XDG_CONFIG_HOME=str(directory / "config"), XDG_STATE_HOME=str(directory / "state"))
        env.pop("DISPLAY", None)
        env.pop("WAYLAND_DISPLAY", None)
        config = directory / "test.conf"
        config.write_text(smoke.CONFIG)

        def spawn(command, **kwargs):
            process = subprocess.Popen([str(arg) for arg in command], env=env, **kwargs)
            processes.append(process)
            return process

        def ctl(*arguments):
            return subprocess.check_output([str(args.ctl), "ctl", *arguments], env=env, text=True, timeout=8)

        def state(*arguments):
            return json.loads(ctl(*arguments, "--json"))

        def test(method, *arguments):
            return subprocess.check_output(["gdbus", "call", "--session", "--dest", "org.freedesktop.portal.Desktop",
                                            "--object-path", "/org/freedesktop/portal/desktop", "--method",
                                            f"org.macafon.Test.{method}", *arguments], env=env, text=True, timeout=8)

        def returned(text):
            return json.loads(text.strip()[2:-3].encode().decode("unicode_escape"))

        try:
            portal = spawn([sys.executable, Path(__file__).with_name("fake_portal.py")],
                           stdout=subprocess.PIPE, text=True)
            assert portal.stdout.readline().strip() == "ready"
            spawn(["pipewire", "-c", config], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            smoke.wait_for(lambda: (runtime / "pipewire-0").exists(), "private PipeWire")
            log = (directory / "daemon.log").open("w")
            spawn([args.daemon, "run", "--no-tray"], stdout=log, stderr=log)
            smoke.wait_for((runtime / "macafon/macafond.sock").exists, "daemon socket")
            smoke.wait_for(lambda: state("hotkeys")["backend"] == "portal", "portal backend")

            ctl("hotkeys", "set", "mic_mute_toggle", "alt+ctrl+m")
            bound = smoke.wait_for(lambda: returned(test("Bound")), "shortcuts bound")
            assert bound == [{"id": "mic_mute_toggle", "description": "Toggle microphone mute",
                              "trigger": "CTRL+ALT+m"}], bound
            binding = next(b for b in state("hotkeys")["bindings"] if b["action"] == "mic_mute_toggle")
            assert binding["shortcut"] == "Ctrl+Alt+M" and binding["system_trigger"] == "CTRL+ALT+m", binding
            conflict = subprocess.run([str(args.ctl), "ctl", "hotkeys", "set", "bypass_toggle", "Ctrl+Alt+M"],
                                      env=env, capture_output=True, text=True, timeout=8)
            assert conflict.returncode != 0 and "already used" in conflict.stderr, conflict
            print("hotkeys: portal binds the preferred trigger; duplicates are refused (HK-002, HK-003)")

            test("Press", "mic_mute_toggle")
            smoke.wait_for(lambda: state("status")["settings"]["muted"], "mute toggled by hotkey")
            test("Release", "mic_mute_toggle")
            test("Press", "mic_mute_toggle")
            smoke.wait_for(lambda: not state("status")["settings"]["muted"], "unmuted by hotkey")

            ctl("hotkeys", "set", "push_to_talk", "F9")
            smoke.wait_for(lambda: len(returned(test("Bound"))) == 2, "push-to-talk bound")
            test("Press", "push_to_talk")
            smoke.wait_for(lambda: not state("status")["settings"]["muted"], "talking while held")
            test("Release", "push_to_talk")
            smoke.wait_for(lambda: state("status")["settings"]["muted"], "muted on release")
            print("hotkeys: toggle mute and push-to-talk hold (HK-001)")

            ctl("hotkeys", "set", "preset_next", "Ctrl+Alt+N")
            ctl("hotkeys", "set", "channel_volume_up.voice", "Ctrl+Alt+Up")
            smoke.wait_for(lambda: len(returned(test("Bound"))) == 4, "four shortcuts bound")
            before = state("status")["preset"]
            test("Press", "preset_next")
            smoke.wait_for(lambda: state("status")["preset"] != before, "next preset")
            test("Press", "channel_volume_up.voice")
            smoke.wait_for(lambda: next(c for c in state("mixer")["channels"] if c["id"] == "voice")
                           ["settings"]["volume_pct"] == 55, "voice volume up")
            notes = smoke.wait_for(lambda: (lambda n: n if len(n) >= 2 else None)(returned(test("Notifications"))),
                                   "OSD notifications")
            assert any(note.startswith("Preset: ") for note in notes) and "Voice: 55%" in notes, notes
            ctl("preset", "previous")
            assert state("status")["preset"] == before, "CLI preset previous"
            print("hotkeys: preset and channel volume with OSD notifications (HK-006); CLI preset previous")
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
    return 0


if __name__ == "__main__":
    sys.exit(main())
