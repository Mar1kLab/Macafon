#!/usr/bin/env python3
"""Exercise actual WirePlumber stream moves in an isolated PipeWire/D-Bus session."""

import argparse
import array
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("smoke", Path(__file__).with_name("pipewire-smoke.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)

SECOND_SINK = """
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = test_voice
        node.description = "Test Voice"
        media.class = Audio/Sink
        audio.position = [ FL FR ]
    } }
]\n
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=Path, default=ROOT / "target/release/macafond")
    parser.add_argument("--ctl", type=Path, default=ROOT / "target/release/macafon")
    parser.add_argument("--private-session", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    for command in ("dbus-run-session", "pipewire", "wireplumber", "wpctl", "pw-cat", "pw-dump"):
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
    with tempfile.TemporaryDirectory(prefix="macafon-routing-") as temporary:
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
        ).replace("node.name = test_sink", "node.name = test_main").rstrip()[:-1] + SECOND_SINK)

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

        def linked_to(stream_id, target_id):
            targets = {item["info"]["input-node-id"] for item in objects()
                       if item["type"] == "PipeWire:Interface:Link"
                       and item["info"]["output-node-id"] == stream_id}
            return targets == {target_id}

        def stop(process):
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
                process.wait(timeout=5)

        try:
            spawn(["pipewire", "-c", config])
            smoke.wait_for(lambda: (runtime / "pipewire-0").exists(), "routing PipeWire server")
            wp = spawn(["wireplumber", "--profile", "macafon-test"])
            smoke.wait_for(lambda: any(item.get("props", {}).get("metadata.name") == "default"
                                      for item in objects()), "WirePlumber policy")
            source_file = directory / "audio.raw"
            samples = array.array("f", [0.0]) * (48000 * 30)
            for index in range(24000, len(samples), 24000):
                samples[index] = 0.5
            if sys.byteorder != "little":
                samples.byteswap()
            source_file.write_bytes(samples.tobytes())
            sinks = nodes()
            for blocked_property in (None, "node.dont-reconnect", "node.dont-move"):
                policy_props = f"{blocked_property}=true" if blocked_property else "node.dont-reconnect=false"
                app = spawn(["pw-cat", "--playback", "--raw", "--format", "f32", "--channels", "1",
                             "--channel-map", "MONO", "--target", "test_main", "--latency", "128",
                             "--properties", f"node.name=test_app {policy_props}", source_file])
                stream = smoke.wait_for(lambda: nodes().get("test_app"), "playback stream")
                stream_id = stream["id"]
                smoke.wait_for(lambda: linked_to(stream_id, sinks["test_main"]["id"]), "initial Main link")
                if blocked_property:
                    attempted = subprocess.run([str(args.daemon), "route", str(stream_id), "test_voice"],
                                               env=env, capture_output=True, text=True, timeout=8)
                    assert attempted.returncode != 0 and f"{blocked_property}=true" in attempted.stderr, attempted
                    assert linked_to(stream_id, sinks["test_main"]["id"]), "blocked stream was disturbed"
                    stop(app)
                    smoke.wait_for(lambda: "test_app" not in nodes(), "blocked stream cleanup")
                    print(f"routing: {blocked_property}=true rejected explicitly; original links retained")
                    continue
                for target in ("test_voice", "test_main", "test_voice"):
                    run(args.daemon, "route", stream_id, target)
                    smoke.wait_for(lambda: linked_to(stream_id, sinks[target]["id"]), f"stream move to {target}")
                    current = nodes()["test_app"]
                    assert current["id"] == stream_id, "stream was recreated"
                    assert current["info"]["props"]["object.serial"] == stream["info"]["props"]["object.serial"], "stream identity changed"
                    assert app.poll() is None and wp.poll() is None, "application or WirePlumber exited"
                run("wpctl", "settings", "linking.allow-moving-streams", "false")
                attempted = subprocess.run([str(args.daemon), "route", str(stream_id), "test_main"],
                                           env=env, capture_output=True, text=True, timeout=8)
                assert attempted.returncode != 0 and "did not move" in attempted.stderr, attempted
                assert linked_to(stream_id, sinks["test_voice"]["id"]), "failed move changed original links"
                run("wpctl", "settings", "linking.allow-moving-streams", "true")
                time.sleep(0.1)
                assert linked_to(stream_id, sinks["test_voice"]["id"]), "failed move left a stale target"
                stop(app)
                smoke.wait_for(lambda: "test_app" not in nodes(), "stream cleanup")
                print("routing: Main → Voice → Main → Voice without stream restart")
                print("routing: disabled stream moving detected; previous Voice target restored")
            microphone = spawn([args.daemon, "run", "--no-tray", "--input", "test_microphone", "--preset", "raw"])
            virtual = smoke.wait_for(lambda: nodes().get("macafon_mic"), "virtual microphone with WirePlumber")
            source = spawn(["pw-cat", "--playback", "--raw", "--format", "f32", "--channels", "1",
                            "--channel-map", "MONO", "--target", "0", "--latency", "128",
                            "--properties", "node.name=test_microphone media.class=Audio/Source adapter.auto-port-config={ mode=dsp position=preserve }", source_file])
            capture = directory / "microphone.raw"
            consumer = spawn(["pw-cat", "--record", "--raw", "--format", "f32", "--channels", "1",
                              "--channel-map", "MONO", "--target", "macafon_mic", "--latency", "128",
                              "--properties", "node.name=test_consumer", capture])
            consumer_node = smoke.wait_for(lambda: nodes().get("test_consumer"), "named microphone consumer")
            smoke.wait_for(lambda: linked_to(virtual["id"], consumer_node["id"]), "WirePlumber microphone capture link")
            time.sleep(1.2)
            stop(consumer)
            captured = array.array("f")
            captured.frombytes(capture.read_bytes())
            if sys.byteorder != "little":
                captured.byteswap()
            assert any(abs(value) > 0.2 for value in captured), "microphone consumer received no signal"
            print("microphone: WirePlumber links a consumer to Macafon Microphone and delivers audio")

            run(args.ctl, "ctl", "ab", "record", "--seconds", "3")
            for device, sink in (("test_voice", "test_voice"), (None, None)):
                run(args.ctl, "ctl", "ab", "play", *(["--device", device] if device else []))
                player = smoke.wait_for(lambda: nodes().get("macafon_ab"), "A/B stream")
                sink_ids = [sinks[sink]["id"]] if sink else [sinks[n]["id"] for n in ("test_main", "test_voice")]
                smoke.wait_for(lambda: any(linked_to(player["id"], target) for target in sink_ids),
                               f"A/B stream linked to {sink or 'the default output'}")
                run(args.ctl, "ctl", "ab", "stop")
                smoke.wait_for(lambda: "macafon_ab" not in nodes(), "A/B stream removed")
            print("ab: WirePlumber links the A/B stream to the chosen and the default output")
            stop(source)
            stop(microphone)
            assert microphone.returncode == 0, "microphone shutdown failed"
        except Exception:
            artifact = ROOT / ".artifacts/wireplumber-routing-failure.json"
            artifact.parent.mkdir(parents=True, exist_ok=True)
            try:
                artifact.write_text(json.dumps(objects(), indent=2))
            except (subprocess.SubprocessError, OSError):
                pass
            for log in logs:
                log.flush()
                log.seek(0)
                print(log.read(), file=sys.stderr)
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
