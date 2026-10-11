#!/usr/bin/env python3
"""M2 mixer in an isolated PipeWire/WirePlumber/D-Bus session: channels, routing, outputs.

Checks: channel nodes appear as sinks and link to the output device (ADR-0003); the default
output becomes the default channel and comes back (OUT-005); apps are classified (APP-006),
moved without restart and remembered (APP-004/005); audio reaches the device; the output
can be switched (OUT-001); turning the mixer off or quitting leaves the graph clean.
"""

import argparse
import array
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("smoke", Path(__file__).with_name("pipewire-smoke.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)

SINKS = """
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = test_speakers
        node.description = "Test Speakers"
        media.class = Audio/Sink
        audio.position = [ FL FR ]
        monitor.channel-volumes = true
    } }
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = test_headphones
        node.description = "Test Headphones"
        media.class = Audio/Sink
        audio.position = [ FL FR ]
        device.form-factor = headphone
    } }
]
"""
PROFILE = """
wireplumber.profiles = {
    macafon-test = {
        inherits = [ policy mixin.stateless ]
        support.snapd-permission-prompting = disabled
        support.portal-permissionstore = disabled
        support.logind = disabled
    }
}
"""


class Client:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(7)
        self.socket.connect(str(path))
        self.reader = self.socket.makefile("r")
        self.sequence = 0
        self.request("hello", {"protocol": 1, "client": "macafon-cli"})

    def request(self, method, params=None, error=False):
        self.sequence += 1
        message = {"id": self.sequence, "method": method}
        if params is not None:
            message["params"] = params
        self.socket.sendall((json.dumps(message) + "\n").encode())
        while True:
            line = self.reader.readline()
            assert line, "daemon closed the connection"
            reply = json.loads(line)
            if reply.get("id") == self.sequence:
                if error:
                    assert "error" in reply, reply
                    return reply["error"]
                assert "error" not in reply, reply
                return reply["result"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=Path, default=ROOT / "target/release/macafond")
    parser.add_argument("--ctl", type=Path, default=ROOT / "target/release/macafon")
    parser.add_argument("--private-session", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    for command in ("dbus-run-session", "pipewire", "wireplumber", "pw-cat", "pw-dump", "pw-loopback"):
        if not shutil.which(command):
            parser.error(f"{command} missing")
    if not args.daemon.is_file() or not args.ctl.is_file():
        parser.error("build first: cargo build --release -p macafond -p macafon-cli")
    if not args.private_session:
        return subprocess.call(["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                                "--private-session", "--daemon", str(args.daemon.resolve()),
                                "--ctl", str(args.ctl.resolve())])
    locations = os.environ.get("WIREPLUMBER_CONFIG_DIR", "/usr/share/wireplumber").split(":")
    default_config = next((Path(location) / "wireplumber.conf" for location in locations
                           if (Path(location) / "wireplumber.conf").is_file()), None)
    if default_config is None:
        parser.error("WirePlumber's distribution config not found")
    processes, logs = [], []
    with tempfile.TemporaryDirectory(prefix="macafon-mixer-") as temporary:
        directory = Path(temporary)
        runtime = directory / "runtime"
        runtime.mkdir(mode=0o700)
        config_directory = directory / "config"
        config_directory.mkdir()
        (config_directory / "wireplumber.conf").write_text(default_config.read_text() + PROFILE)
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), PIPEWIRE_RUNTIME_DIR=str(runtime),
                   XDG_CONFIG_HOME=str(config_directory), XDG_STATE_HOME=str(directory / "state"),
                   WIREPLUMBER_CONFIG_DIR=str(config_directory))
        config = directory / "pipewire.conf"
        config.write_text(smoke.CONFIG.replace(
            '    { factory = metadata args = { metadata.name = default } }\n', ""
        ).replace("node.name = test_sink", "node.name = test_unused").rstrip()[:-1] + SINKS)

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

        def default_sink():
            for item in objects():
                if item["type"] == "PipeWire:Interface:Metadata" and \
                        item.get("props", {}).get("metadata.name") == "default":
                    for entry in item.get("metadata", []):
                        if entry["subject"] == 0 and entry["key"] == "default.configured.audio.sink":
                            value = entry["value"]
                            return (json.loads(value) if isinstance(value, str) else value).get("name")
            return None

        def stop(process):
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
                process.wait(timeout=7)

        def app(name, binary, audio):
            return spawn(["pw-cat", "--playback", "--raw", "--format", "f32", "--channels", "2",
                          "--latency", "128", "--properties",
                          f"node.name={name} application.name={binary} application.process.binary={binary}",
                          audio])

        try:
            server = spawn(["pipewire", "-c", config])
            smoke.wait_for(lambda: (runtime / "pipewire-0").exists(), "mixer PipeWire server")
            wp = spawn(["wireplumber", "--profile", "macafon-test"])
            smoke.wait_for(lambda: any(item.get("props", {}).get("metadata.name") == "default"
                                      for item in objects()), "WirePlumber policy")
            run("pw-metadata", "-n", "default", "0", "default.configured.audio.sink",
                json.dumps({"name": "test_speakers"}), "Spa:String:JSON")
            smoke.wait_for(lambda: default_sink() == "test_speakers", "initial default output")
            audio = directory / "audio.raw"
            samples = array.array("f", [0.0]) * (48000 * 2 * 30)
            for index in range(0, len(samples), 2):
                samples[index] = samples[index + 1] = 0.5 if (index // 2) % 96 < 48 else -0.5
            if sys.byteorder != "little":
                samples.byteswap()
            audio.write_bytes(samples.tobytes())

            daemon = spawn([args.daemon, "run", "--no-tray"])
            socket_path = runtime / "macafon/macafond.sock"
            smoke.wait_for(socket_path.exists, "daemon socket")
            client = Client(socket_path)
            state = smoke.wait_for(lambda: (lambda s: s if s["outputs"] else None)(
                client.request("mixer.get")), "output devices reported")
            assert not state["enabled"] and [c["id"] for c in state["channels"]] == ["main", "voice", "music"]
            names = {output["name"] for output in state["outputs"]}
            assert names >= {"test_speakers", "test_headphones"} and not any(
                name.startswith("macafon_") for name in names), state["outputs"]

            state = client.request("mixer.set_enabled", {"enabled": True})
            assert state["enabled"]
            for channel in ("main", "voice", "music"):
                node = smoke.wait_for(lambda: nodes().get(f"macafon_ch_{channel}"), f"{channel} node")
                assert node["info"]["props"]["media.class"] == "Audio/Sink"
                speakers = nodes()["test_speakers"]["id"]
                smoke.wait_for(lambda: targets(node["id"]) == {speakers}, f"{channel} linked to the output")
            smoke.wait_for(lambda: default_sink() == "macafon_ch_main", "default output is Main")
            print("mixer: three channel sinks linked to the output; default output is Macafon Main")

            voice_node = nodes()["macafon_ch_voice"]["id"]
            main_node = nodes()["macafon_ch_main"]["id"]
            music_node = nodes()["macafon_ch_music"]["id"]
            discord = app("test_discord", "Discord", audio)
            stream = smoke.wait_for(lambda: nodes().get("test_discord"), "Discord stream")
            smoke.wait_for(lambda: targets(stream["id"]) == {voice_node}, "Discord classified into Voice")
            firefox = app("test_firefox", "firefox", audio)
            browser = smoke.wait_for(lambda: nodes().get("test_firefox"), "Firefox stream")
            smoke.wait_for(lambda: targets(browser["id"]) == {main_node}, "Firefox in Main")
            keys = smoke.wait_for(lambda: (lambda apps: apps if "bin:firefox" in apps else None)(
                {entry["key"]: entry for entry in client.request("mixer.get")["apps"]}), "apps reported")
            assert keys["bin:discord"]["channel"] == "voice" and keys["bin:firefox"]["channel"] == "main", keys
            print("apps: Discord classified into Voice, Firefox into Main by default")

            client.request("apps.move", {"app_key": "bin:firefox", "channel": "music"})
            smoke.wait_for(lambda: targets(browser["id"]) == {music_node}, "Firefox moved to Music")
            assert nodes()["test_firefox"]["id"] == browser["id"] and firefox.poll() is None, "stream restarted"
            stop(firefox)
            smoke.wait_for(lambda: "test_firefox" not in nodes(), "Firefox closed")
            firefox = app("test_firefox", "firefox", audio)
            browser = smoke.wait_for(lambda: nodes().get("test_firefox"), "Firefox again")
            smoke.wait_for(lambda: targets(browser["id"]) == {music_node}, "rule applied on restart")
            print("apps: moved without restart and remembered for the next launch")

            client.request("channel.set", {"id": "music", "settings": {
                **next(c for c in state["channels"] if c["id"] == "music")["settings"], "volume_pct": 100.0}})
            capture = directory / "output.raw"
            recorder = spawn(["pw-cat", "--record", "--raw", "--format", "f32", "--channels", "2",
                              "--target", "test_speakers", "--latency", "128",
                              "--properties", "node.name=test_monitor stream.capture.sink=true", capture])
            smoke.wait_for(lambda: nodes().get("test_monitor"), "output monitor")
            time.sleep(1.0)
            stop(recorder)
            captured = array.array("f")
            captured.frombytes(capture.read_bytes()[: len(capture.read_bytes()) // 4 * 4])
            if sys.byteorder != "little":
                captured.byteswap()
            peak = max((abs(value) for value in captured), default=0.0)
            assert peak > 0.1, f"no audio from the channels at the output device (peak {peak})"
            print(f"audio: channels reach the output device (peak {peak:.2f})")

            run(args.ctl, "ctl", "channel", "voice", "volume", "+5")
            run(args.ctl, "ctl", "channel", "music", "volume", "40")
            run(args.ctl, "ctl", "channel", "main", "mute")
            volumes = {c["id"]: c["settings"] for c in client.request("mixer.get")["channels"]}
            assert volumes["voice"]["volume_pct"] == 55 and volumes["music"]["volume_pct"] == 40, volumes
            assert volumes["main"]["muted"], volumes
            print("cli: channel volume and mute commands for desktop hotkeys (HK-004)")

            report = client.request("doctor.run")
            statuses = {check["id"]: check["status"] for check in report["checks"]}
            assert statuses["pipewire"] == "ok" and statuses["session_manager"] == "ok", report
            assert statuses["mixer"] == "ok" and statuses["microphone_input"] == "warning", report
            assert report["quantum"] == 128 and report["sample_rate_hz"] == 48000, report
            assert "Pipewire: 1.6" in run(args.ctl, "ctl", "doctor") or "Pipewire" in run(args.ctl, "ctl", "doctor")
            print("doctor: versions, mixer links, quantum and microphone state reported")

            client.request("mixer.set_output", {"device": "test_headphones"})
            headphones = nodes()["test_headphones"]["id"]
            smoke.wait_for(lambda: targets(music_node) == {headphones}, "channels moved to headphones")
            print("outputs: switching the output relinks every channel")

            # OUT-002: a preferred device wins while present; unplugged, the next one takes over.
            client.request("mixer.set_output_priority", {"devices": ["test_usb", "test_speakers"]})
            speakers = nodes()["test_speakers"]["id"]
            smoke.wait_for(lambda: targets(music_node) == {speakers}, "next device in priority")
            for _ in range(2):
                usb = spawn(["pw-loopback", "--capture-props",
                             "media.class=Audio/Sink node.name=test_usb node.description=USB audio.position=[FL FR]",
                             "--playback-props", "node.name=test_usb_out target.object=test_unused"])
                usb_node = smoke.wait_for(lambda: nodes().get("test_usb"), "USB device plugged")
                smoke.wait_for(lambda: targets(music_node) == {usb_node["id"]}, "preferred device taken")
                stop(usb)
                smoke.wait_for(lambda: targets(music_node) == {speakers}, "fallback after unplug")
            print("outputs: priority list follows plug and unplug of the preferred device")

            client.request("mixer.set_enabled", {"enabled": False})
            smoke.wait_for(lambda: not any(name and name.startswith("macafon_ch_") for name in nodes()),
                           "channel nodes removed")
            smoke.wait_for(lambda: default_sink() == "test_speakers", "default output restored")
            smoke.wait_for(lambda: targets(browser["id"]) and not targets(browser["id"]) & {
                voice_node, main_node, music_node}, "apps back on a real device")
            assert discord.poll() is None and firefox.poll() is None, "apps exited"
            print("mixer off: channels gone, default output restored, apps keep playing")

            client.request("mixer.set_enabled", {"enabled": True})
            smoke.wait_for(lambda: default_sink() == "macafon_ch_main", "default output taken again")

            # NFR-S8: the channels come back after the audio server restarts.
            stop(wp)
            stop(server)
            server = spawn(["pipewire", "-c", config])
            smoke.wait_for(lambda: (runtime / "pipewire-0").exists(), "restarted PipeWire")
            wp = spawn(["wireplumber", "--profile", "macafon-test"])
            for channel in ("main", "voice", "music"):
                smoke.wait_for(lambda: nodes().get(f"macafon_ch_{channel}"), f"{channel} after restart")
            restored = nodes()
            smoke.wait_for(lambda: targets(restored["macafon_ch_main"]["id"]) == {restored["test_speakers"]["id"]},
                           "channels relinked after restart")
            smoke.wait_for(lambda: default_sink() == "macafon_ch_main", "default output after restart")
            print("restart: channels, links and the default output survive a PipeWire restart")
            client = Client(socket_path)
            client.request("daemon.quit")
            daemon.wait(timeout=7)
            assert daemon.returncode == 0, "daemon shutdown failed"
            smoke.wait_for(lambda: default_sink() == "test_speakers", "default output restored on quit")
            assert not any(name and name.startswith("macafon_") for name in nodes()), "nodes left behind"
            assert wp.poll() is None, "WirePlumber exited"
            print("quit: default output restored and no Macafon nodes left")
        except Exception:
            artifact = ROOT / ".artifacts/mixer-smoke-failure.json"
            artifact.parent.mkdir(parents=True, exist_ok=True)
            try:
                artifact.write_text(json.dumps(objects(), indent=2))
            except (subprocess.SubprocessError, OSError):
                pass
            saved = ROOT / ".artifacts/mixer-smoke-logs"
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
