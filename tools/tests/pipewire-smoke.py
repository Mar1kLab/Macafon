#!/usr/bin/env python3
"""Local M0 integration test in a private PipeWire graph, without physical hardware."""

import argparse
import array
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("latency", ROOT / "tools/latency-probe/analyze.py")
latency = importlib.util.module_from_spec(spec)
spec.loader.exec_module(latency)

CONFIG = """
context.properties = {
    core.daemon = true
    core.name = pipewire-0
    default.clock.rate = 48000
    default.clock.allowed-rates = [ 48000 ]
    default.clock.quantum = 128
    default.clock.min-quantum = 128
    default.clock.max-quantum = 128
}
context.spa-libs = {
    audio.convert.* = audioconvert/libspa-audioconvert
    support.* = support/libspa-support
}
context.modules = [
    { name = libpipewire-module-protocol-native }
    { name = libpipewire-module-metadata }
    { name = libpipewire-module-spa-node-factory }
    { name = libpipewire-module-client-node }
    { name = libpipewire-module-access }
    { name = libpipewire-module-adapter }
    { name = libpipewire-module-link-factory }
]
context.objects = [
    { factory = spa-node-factory args = {
        factory.name = support.node.driver
        node.name = Dummy-Driver
        node.group = pipewire.dummy
        priority.driver = 20000
    } }
    { factory = metadata args = { metadata.name = default } }
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = test_sink
        node.description = "Test sink"
        media.class = Audio/Sink
        audio.position = [ FL FR ]
    } }
]
"""


def wait_for(predicate, description):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.05)
    raise RuntimeError(f"Timed out: {description}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=Path, default=ROOT / "target/release/miceqd")
    args = parser.parse_args()
    for command in ("pipewire", "pw-cat", "pw-link", "pw-dump", "pw-metadata"):
        if not shutil.which(command):
            parser.error(f"{command} missing; install PipeWire tools")
    if not args.daemon.is_file():
        parser.error("build the daemon first: cargo build --release -p miceqd")
    processes = []
    with tempfile.TemporaryDirectory(prefix="miceq-pw-") as directory:
        directory = Path(directory)
        runtime = directory / "runtime"
        runtime.mkdir(mode=0o700)
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), PIPEWIRE_RUNTIME_DIR=str(runtime),
                   XDG_CONFIG_HOME=str(directory / "config"))
        config = directory / "test.conf"
        config.write_text(CONFIG)
        logs = []

        def spawn(command, **kwargs):
            log = (directory / f"process-{len(logs)}.log").open("w+")
            logs.append(log)
            process = subprocess.Popen([str(arg) for arg in command], env=env,
                                       stderr=log, stdout=log, **kwargs)
            processes.append(process)
            return process

        def run(*command):
            return subprocess.check_output([str(arg) for arg in command], env=env, text=True,
                                           stderr=subprocess.PIPE, timeout=8)

        def objects():
            return json.loads(run("pw-dump"))

        def ports(name, direction):
            data = objects()
            node_ids = {item["id"] for item in data
                        if item.get("info", {}).get("props", {}).get("node.name") == name}
            return {item["info"]["props"].get("audio.channel"): item["id"] for item in data
                    if item["type"] == "PipeWire:Interface:Port"
                    and int(item["info"]["props"].get("node.id", -1)) in node_ids
                    and item["info"]["props"].get("port.direction") == direction}

        def stop(process):
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
                process.wait(timeout=5)

        server = None
        try:
            server = spawn(["pipewire", "-c", config])
            wait_for(lambda: (runtime / "pipewire-0").exists(), "private PipeWire server")
            source_file = directory / "impulses.raw"
            source = array.array("f", [0.0]) * (48000 * 8)
            for index in range(24000, len(source), 24000):
                source[index] = 0.5
            if sys.byteorder != "little":
                source.byteswap()
            source_file.write_bytes(source.tobytes())

            for engine in ("raw", "lite", "mute", "natural"):
                command = [args.daemon, "run", "--input", "test_microphone"]
                if engine != "natural":
                    command += ["--preset", "raw"]
                if engine == "lite":
                    command += ["--engine", "lite", "--intensity", "0"]
                if engine == "mute":
                    command += ["--mute"]
                daemon = spawn(command)
                wait_for(lambda: ports("miceq_mic", "out"), "virtual microphone")
                capture = directory / f"{engine}.raw"
                recorder = spawn(["pw-cat", "--record", "--raw", "--format", "f32",
                                  "--channels", "2", "--channel-map", "FL,FR", "--target", "0",
                                  "--latency", "128", "--properties", "node.name=test_capture adapter.auto-port-config={ mode=dsp position=preserve }", capture])
                capture_ports = wait_for(lambda: ports("test_capture", "in"), "capture ports")
                source_command = ["pw-cat", "--playback", "--raw", "--format", "f32",
                                        "--channels", "1", "--channel-map", "MONO", "--target", "0",
                                        "--latency", "128", "--properties",
                                        "node.name=test_microphone media.class=Audio/Source adapter.auto-port-config={ mode=dsp position=preserve }", source_file]
                source_process = spawn(source_command)
                source_ports = wait_for(lambda: ports("test_microphone", "out"), "source ports")
                virtual_ports = ports("miceq_mic", "out")
                run("pw-link", str(source_ports["MONO"]), str(capture_ports["FL"]))
                run("pw-link", str(virtual_ports["MONO"]), str(capture_ports["FR"]))
                # The daemon links the named input itself, including after reconnection.
                time.sleep(1.2)
                if daemon.poll() is not None:
                    raise RuntimeError(f"daemon exited during {engine}: {daemon.returncode}")
                stop(source_process)
                time.sleep(0.2)
                source_process = spawn(source_command)
                new_source_ports = wait_for(lambda: ports("test_microphone", "out"), "reconnected source")
                run("pw-link", str(new_source_ports["MONO"]), str(capture_ports["FL"]))
                time.sleep(1.2)
                stop(source_process)
                time.sleep(0.15)
                stop(recorder)
                samples = array.array("f")
                samples.frombytes(capture.read_bytes())
                if sys.byteorder != "little":
                    samples.byteswap()
                if not all(math.isfinite(value) for value in samples):
                    raise AssertionError("non-finite audio")
                if engine == "mute":
                    assert any(abs(value) > 0.2 for value in samples[0::2]), "no reference signal"
                    assert all(value == 0.0 for value in samples[1::2]), "mute leaked audio"
                    print("mute: digital silence with active input")
                elif engine == "natural":
                    processed = samples[1::2]
                    ceiling = 10 ** (-1 / 20)
                    assert any(abs(value) > 1e-3 for value in processed), "Natural chain produced no audio"
                    assert all(abs(value) <= ceiling for value in processed), "Natural exceeded the limiter ceiling"
                    print(f"natural: full chain passes audio, peak {max(map(abs, processed)):.3f} <= {ceiling:.3f}")
                else:
                    artifacts = ROOT / ".artifacts/latency"
                    artifacts.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(capture, artifacts / f"{engine}.raw")
                    result = latency.analyze(capture)
                    expected = 128 if engine == "raw" else 1088
                    assert result["added_latency_samples"] == expected, result
                    processed = samples[1::2]
                    assert any(abs(value) > 0.2 for value in processed[len(processed) // 2:]), "source did not reconnect"
                    print(f"{engine}: {json.dumps(result)}")
                assert all(value == 0.0 for value in samples[1::2][-2048:]), "disconnected input leaked audio"
                stop(daemon)
                assert daemon.returncode == 0, f"unclean shutdown: {daemon.returncode}"
                wait_for(lambda: not ports("miceq_mic", "out"), "virtual microphone cleanup")
            playback = spawn(["pw-cat", "--playback", "--raw", "--format", "f32",
                              "--channels", "1", "--channel-map", "MONO", "--target", "0",
                              "--properties", "node.name=test_app adapter.auto-port-config={ mode=dsp position=preserve }", source_file])
            wait_for(lambda: ports("test_app", "out"), "playback stream")
            nodes = {item["info"]["props"].get("node.name"): item for item in objects()
                     if item["type"] == "PipeWire:Interface:Node"}
            attempted = subprocess.run([str(args.daemon), "route", str(nodes["test_app"]["id"]), "test_sink"],
                                       env=env, capture_output=True, text=True, timeout=8)
            assert attempted.returncode != 0 and "did not move" in attempted.stderr, attempted
            metadata = run("pw-metadata", "-n", "default")
            assert "target.object" not in metadata, "failed move left stale routing metadata"
            print("routing: missing policy detected; failed move restores empty target metadata")
            stop(playback)
            # Rebuild the whole graph after server loss, preserving the selected chain.
            daemon = spawn([args.daemon, "run", "--input", "test_microphone", "--preset", "raw",
                            "--engine", "lite", "--intensity", "0"])
            wait_for(lambda: ports("miceq_mic", "out"), "virtual microphone before server exit")
            for _ in range(2):
                stop(server)
                time.sleep(0.7)
                assert daemon.poll() is None, "daemon exited instead of waiting for PipeWire"
                server = spawn(["pipewire", "-c", config])
                wait_for(lambda: (runtime / "pipewire-0").exists(), "restarted server")
                wait_for(lambda: ports("miceq_mic", "out"), "restored virtual microphone")
                restored = [item for item in objects() if item["type"] == "PipeWire:Interface:Node"
                            and item["info"]["props"].get("node.name") == "miceq_mic"]
                assert len(restored) == 1, "duplicate virtual microphone after reconnect"
                assert restored[0]["info"]["params"]["ProcessLatency"][0]["rate"] == 960, "processing settings lost"
            capture = directory / "recovery.raw"
            recorder = spawn(["pw-cat", "--record", "--raw", "--format", "f32", "--channels", "1",
                              "--channel-map", "MONO", "--target", "0", "--latency", "128",
                              "--properties", "node.name=test_capture adapter.auto-port-config={ mode=dsp position=preserve }", capture])
            capture_ports = wait_for(lambda: ports("test_capture", "in"), "recovery capture")
            source_process = spawn(source_command)
            wait_for(lambda: ports("test_microphone", "out"), "source after server restart")
            run("pw-link", str(ports("miceq_mic", "out")["MONO"]), str(capture_ports["MONO"]))
            time.sleep(1.2)
            stop(source_process)
            stop(recorder)
            samples = array.array("f")
            samples.frombytes(capture.read_bytes())
            if sys.byteorder != "little":
                samples.byteswap()
            assert any(abs(value) > 0.2 for value in samples), "restored graph produced no audio"
            stop(daemon)
            assert daemon.returncode == 0, "restored daemon did not shut down cleanly"
            wait_for(lambda: not ports("miceq_mic", "out"), "restored node cleanup")
            stop(server)
            daemon = spawn([args.daemon])
            time.sleep(0.7)
            assert daemon.poll() is None, "startup without PipeWire should wait"
            daemon.send_signal(signal.SIGINT)
            daemon.wait(timeout=5)
            assert daemon.returncode == 0, "cannot interrupt reconnect wait"
            print("lifecycle: two server restarts preserve Lite settings and audio; startup wait remains interruptible")
        except Exception:
            if server is not None and server.poll() is None:
                print(run("pw-link", "-l"), file=sys.stderr)
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


if __name__ == "__main__":
    main()
