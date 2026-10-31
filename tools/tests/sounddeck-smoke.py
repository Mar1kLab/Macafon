#!/usr/bin/env python3
"""Linux SoundDeck in an isolated PipeWire/WirePlumber/D-Bus session (PRD §7.6).

Checks: a sound in the library folder is found and plays into Macafon Microphone (SND-002);
the headphones monitor plays on the default output, is the "SoundDeck" app of the mixer and
moves between channels (SND-002 on Linux); a chosen headphones device pins the monitor; the
monitor stream goes away after playback.
"""

import argparse
import array
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import wave

ROOT = Path(__file__).resolve().parents[2]


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(file))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


smoke = load("smoke", "pipewire-smoke.py")
mixer = load("mixer_smoke", "mixer-smoke.py")


def write_tone(path, seconds=8.0, freq_hz=440.0):
    frames = int(48000 * seconds)
    samples = array.array("h", [0]) * (frames * 2)
    for index in range(frames):
        value = int(0.5 * 32767 * math.sin(2 * math.pi * freq_hz * index / 48000))
        samples[2 * index] = samples[2 * index + 1] = value
    if sys.byteorder != "little":
        samples.byteswap()
    with wave.open(str(path), "wb") as file:
        file.setnchannels(2)
        file.setsampwidth(2)
        file.setframerate(48000)
        file.writeframes(samples.tobytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=Path, default=ROOT / "target/release/macafond")
    parser.add_argument("--private-session", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    for command in ("dbus-run-session", "pipewire", "wireplumber", "pw-cat", "pw-dump"):
        if not shutil.which(command):
            parser.error(f"{command} missing")
    if not args.daemon.is_file():
        parser.error("build first: cargo build --release -p macafond")
    if not args.private_session:
        return subprocess.call(["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                                "--private-session", "--daemon", str(args.daemon.resolve())])
    locations = os.environ.get("WIREPLUMBER_CONFIG_DIR", "/usr/share/wireplumber").split(":")
    default_config = next((Path(location) / "wireplumber.conf" for location in locations
                           if (Path(location) / "wireplumber.conf").is_file()), None)
    if default_config is None:
        parser.error("WirePlumber's distribution config not found")
    processes, logs = [], []
    with tempfile.TemporaryDirectory(prefix="macafon-sounddeck-") as temporary:
        directory = Path(temporary)
        runtime = directory / "runtime"
        runtime.mkdir(mode=0o700)
        home = directory / "home"
        library = home / "Music/Macafon Sounds"
        library.mkdir(parents=True)
        write_tone(library / "tone.wav")
        config_directory = directory / "config"
        config_directory.mkdir()
        (config_directory / "wireplumber.conf").write_text(default_config.read_text() + mixer.PROFILE)
        env = dict(os.environ, HOME=str(home), XDG_RUNTIME_DIR=str(runtime),
                   PIPEWIRE_RUNTIME_DIR=str(runtime), XDG_CONFIG_HOME=str(config_directory),
                   XDG_STATE_HOME=str(directory / "state"), XDG_DATA_HOME=str(directory / "data"),
                   WIREPLUMBER_CONFIG_DIR=str(config_directory))
        config = directory / "pipewire.conf"
        config.write_text(smoke.CONFIG.replace(
            '    { factory = metadata args = { metadata.name = default } }\n', ""
        ).replace("node.name = test_sink", "node.name = test_unused").rstrip()[:-1] + mixer.SINKS)

        def spawn(command):
            log = (directory / f"process-{len(logs)}.log").open("w+")
            logs.append(log)
            process = subprocess.Popen([str(arg) for arg in command], env=env, stdout=log, stderr=log)
            processes.append(process)
            return process

        def run(*command):
            return subprocess.check_output([str(arg) for arg in command], env=env,
                                           text=True, stderr=subprocess.PIPE, timeout=8)

        def objects():
            return json.loads(run("pw-dump"))

        def nodes():
            return {item["info"]["props"].get("node.name"): item for item in objects()
                    if item["type"] == "PipeWire:Interface:Node"}

        def targets(node_id):
            return {item["info"]["input-node-id"] for item in objects()
                    if item["type"] == "PipeWire:Interface:Link"
                    and item["info"]["output-node-id"] == node_id}

        def stop(process):
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=7)

        def record_peak(target, monitor=False):
            capture = directory / f"capture-{len(logs)}.raw"
            props = "node.name=test_recorder" + (" stream.capture.sink=true" if monitor else "")
            recorder = spawn(["pw-cat", "--record", "--raw", "--format", "f32", "--channels", "2",
                              "--target", target, "--latency", "128", "--properties", props, capture])
            smoke.wait_for(lambda: nodes().get("test_recorder"), f"recorder on {target}")
            time.sleep(1.0)
            stop(recorder)
            captured = array.array("f")
            data = capture.read_bytes()
            captured.frombytes(data[: len(data) // 4 * 4])
            if sys.byteorder != "little":
                captured.byteswap()
            return max((abs(value) for value in captured), default=0.0)

        try:
            spawn(["pipewire", "-c", config])
            smoke.wait_for(lambda: (runtime / "pipewire-0").exists(), "PipeWire server")
            spawn(["wireplumber", "--profile", "macafon-test"])
            smoke.wait_for(lambda: any(item.get("props", {}).get("metadata.name") == "default"
                                      for item in objects()), "WirePlumber policy")
            run("pw-metadata", "-n", "default", "0", "default.configured.audio.sink",
                json.dumps({"name": "test_speakers"}), "Spa:String:JSON")

            daemon = spawn([args.daemon, "run", "--no-tray"])
            socket_path = runtime / "macafon/macafond.sock"
            smoke.wait_for(socket_path.exists, "daemon socket")
            client = mixer.Client(socket_path)
            assert client.request("hello", {"protocol": 1, "client": "macafon-cli"})["capabilities"]["sounddeck"]
            state = smoke.wait_for(lambda: (lambda s: s if s["sounds"] else None)(
                client.request("sounddeck.get")), "library scanned")
            assert [sound["id"] for sound in state["sounds"]] == ["tone.wav"], state["sounds"]
            assert state["microphone_ready"], state
            print(f"library: {state['folder']} scanned, tone.wav found")

            client.request("sounddeck.play", {"id": "tone.wav"})
            smoke.wait_for(lambda: nodes().get("macafon_sounddeck"), "monitor stream")
            peak = record_peak("macafon_mic")
            assert peak > 0.05, f"no SoundDeck in Macafon Microphone (peak {peak})"
            print(f"microphone: the sound plays into Macafon Microphone (peak {peak:.2f})")
            monitor = nodes()["macafon_sounddeck"]
            smoke.wait_for(lambda: targets(monitor["id"]) == {nodes()["test_speakers"]["id"]},
                           "monitor on the default output")
            peak = record_peak("test_speakers", monitor=True)
            assert peak > 0.01, f"no SoundDeck in the headphones (peak {peak})"
            print(f"monitor: the sound plays on the default output (peak {peak:.2f})")

            client.request("mixer.set_enabled", {"enabled": True})
            main = smoke.wait_for(lambda: nodes().get("macafon_ch_main"), "Main channel")
            smoke.wait_for(lambda: targets(nodes()["macafon_sounddeck"]["id"]) == {main["id"]},
                           "monitor routed into Main")
            apps = {app["key"]: app for app in client.request("mixer.get")["apps"]}
            assert apps.get("name:SoundDeck", {}).get("name") == "SoundDeck", apps
            client.request("apps.move", {"app_key": "name:SoundDeck", "channel": "music"})
            music = nodes()["macafon_ch_music"]
            smoke.wait_for(lambda: targets(nodes()["macafon_sounddeck"]["id"]) == {music["id"]},
                           "monitor moved to Music")
            print("mixer: the monitor is the SoundDeck app and moves between channels")

            client.request("sounddeck.set_monitor_device", {"device": "test_headphones"})
            headphones = nodes()["test_headphones"]["id"]
            smoke.wait_for(lambda: nodes().get("macafon_sounddeck") and targets(
                nodes()["macafon_sounddeck"]["id"]) == {headphones}, "monitor pinned to headphones")
            print("monitor: a chosen headphones device wins over channel routing")

            client.request("sounddeck.stop")
            # The monitor lingers 15 s after the last sound, so a quick next one starts at once.
            deadline = time.monotonic() + 25
            while "macafon_sounddeck" in nodes():
                assert time.monotonic() < deadline, "monitor stream stayed after playback"
                time.sleep(0.5)
            print("stop: the monitor stream goes away after the linger time")

            client.request("daemon.quit")
            daemon.wait(timeout=7)
            assert daemon.returncode == 0, "daemon shutdown failed"
            assert not any(name and name.startswith("macafon_") for name in nodes()), "nodes left behind"
            print("quit: no Macafon nodes left")
        except Exception:
            saved = ROOT / ".artifacts/sounddeck-smoke-logs"
            saved.mkdir(parents=True, exist_ok=True)
            for index, log in enumerate(logs):
                log.flush()
                log.seek(0)
                text = log.read()
                (saved / f"process-{index}.log").write_text(text)
                print(text[-3000:], file=sys.stderr)
            raise
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    try:
                        stop(process)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            for log in logs:
                log.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
