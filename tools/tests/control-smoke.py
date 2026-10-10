#!/usr/bin/env python3
"""Live control of a running daemon through `miceq ctl` in a private PipeWire graph."""

import argparse
import array
import importlib.util
import json
import math
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=Path, default=ROOT / "target/release/miceqd")
    parser.add_argument("--ctl", type=Path, default=ROOT / "target/release/miceq")
    args = parser.parse_args()
    for command in ("pipewire", "pw-cat", "pw-link", "pw-dump"):
        if not shutil.which(command):
            parser.error(f"{command} missing; install PipeWire tools")
    if not args.daemon.is_file() or not args.ctl.is_file():
        parser.error("build first: cargo build --release -p miceqd -p miceq-cli")
    processes, logs = [], []
    with tempfile.TemporaryDirectory(prefix="miceq-control-") as temporary:
        directory = Path(temporary)
        runtime = directory / "runtime"
        runtime.mkdir(mode=0o700)
        config_home = directory / "config"
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), PIPEWIRE_RUNTIME_DIR=str(runtime),
                   XDG_CONFIG_HOME=str(config_home), MICEQ_NO_SYSTEMD="1")
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

        def ctl(*command):
            return run(args.ctl, "ctl", *command)

        def status():
            return json.loads(ctl("status", "--json"))

        def request(method, params=None):
            with socket.socket(socket.AF_UNIX) as connection:
                connection.settimeout(8)
                connection.connect(str(runtime / "miceq" / "miceqd.sock"))
                stream = connection.makefile("rw")
                for identifier, name, arguments in [(1, "hello", {"protocol": 1, "client": "smoke"}),
                                                     (2, method, params)]:
                    message = {"id": identifier, "method": name}
                    if arguments is not None:
                        message["params"] = arguments
                    stream.write(json.dumps(message) + "\n")
                    stream.flush()
                    while True:
                        reply = json.loads(stream.readline())
                        if reply.get("id") == identifier:
                            assert "result" in reply, reply
                            break
                return reply["result"]

        def ports(name, direction):
            data = json.loads(run("pw-dump"))
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

        def record(seconds=0.6, source="miceq_mic"):
            """Capture a node's output; drop the first 100 ms of ramps and linking."""
            capture = directory / "capture.raw"
            recorder = spawn(["pw-cat", "--record", "--raw", "--format", "f32", "--channels", "1",
                              "--channel-map", "MONO", "--target", "0", "--latency", "128",
                              "--properties", "node.name=test_capture adapter.auto-port-config={ mode=dsp position=preserve }",
                              capture])
            capture_ports = smoke.wait_for(lambda: ports("test_capture", "in"), "capture ports")
            run("pw-link", str(ports(source, "out")["MONO"]), str(capture_ports["MONO"]))
            time.sleep(seconds)
            stop(recorder)
            samples = array.array("f")
            samples.frombytes(capture.read_bytes())
            if sys.byteorder != "little":
                samples.byteswap()
            samples = samples[4800:]
            assert len(samples) > 4800, "capture too short"
            assert all(math.isfinite(value) for value in samples), "non-finite audio"
            return max(abs(value) for value in samples)

        def start_daemon(*extra):
            daemon = spawn([args.daemon, "run", "--no-tray", *extra])
            smoke.wait_for(lambda: (runtime / "miceq" / "miceqd.sock").exists(), "control socket")
            smoke.wait_for(lambda: ports("miceq_mic", "out"), "virtual microphone")
            smoke.wait_for(lambda: status()["input_connected"], "microphone linked")
            return daemon

        try:
            server = spawn(["pipewire", "-c", config])
            smoke.wait_for(lambda: (runtime / "pipewire-0").exists(), "private PipeWire server")
            tone = array.array("f", [0.5 * math.sin(2 * math.pi * 440 * n / 48000)
                                     for n in range(48000 * 30)])
            if sys.byteorder != "little":
                tone.byteswap()
            source_file = directory / "tone.raw"
            source_file.write_bytes(tone.tobytes())
            spawn(["pw-cat", "--playback", "--raw", "--format", "f32", "--channels", "1",
                   "--channel-map", "MONO", "--target", "0", "--latency", "128", "--properties",
                   "node.name=test_microphone media.class=Audio/Source adapter.auto-port-config={ mode=dsp position=preserve }",
                   source_file])
            smoke.wait_for(lambda: ports("test_microphone", "out"), "source ports")

            daemon = start_daemon("--input", "test_microphone", "--preset", "raw")
            second = subprocess.run([str(args.daemon), "run", "--no-tray"], env=env, capture_output=True,
                                    text=True, timeout=8)
            assert second.returncode != 0 and "already running" in second.stderr, second
            print("socket: second daemon refused while the first runs")

            peak = record()
            assert 0.45 < peak < 0.55, f"raw peak {peak}"
            ctl("mute", "on")
            assert record() == 0.0, "mute leaked audio"
            ctl("mute", "toggle")
            assert record() > 0.45, "unmute did not restore audio"
            ctl("gain", "-6")
            peak = record()
            assert 0.23 < peak < 0.27, f"−6 dB peak {peak}"
            print(f"control: mute/unmute and gain applied live (−6 dB peak {peak:.3f})")

            # A/B: record 3 s of the raw 0.5 tone; B is the −6 dB chain.
            clip = json.loads(ctl("ab", "record", "--seconds", "3", "--json"))
            assert clip["clip_ms"] == 3000 and not clip["playing"], clip
            state = json.loads(ctl("ab", "play", "--json"))
            assert state["playing"] and state["side"] == "b" and state["loudness_match"], state
            # Without WirePlumber nobody configures the stream's ports; do what it would.
            ab_node = smoke.wait_for(lambda: next(
                (item["id"] for item in json.loads(run("pw-dump"))
                 if item.get("info", {}).get("props", {}).get("node.name") == "miceq_ab"), None),
                "A/B stream node")
            run("pw-cli", "set-param", str(ab_node), "PortConfig",
                "{ direction: Output, mode: dsp, format: { mediaType: audio, mediaSubtype: raw, "
                "format: F32P, rate: 48000, channels: 1, position: [ MONO ] } }")
            smoke.wait_for(lambda: ports("miceq_ab", "out"), "A/B stream ports")
            b_peak = record(1.0, "miceq_ab")
            ctl("gain", "-12")  # AB-003: B follows the live chain
            b_live_peak = record(1.0, "miceq_ab")
            assert 0.11 < b_live_peak < 0.14, f"B after a live change {b_live_peak}"
            ctl("gain", "-6")
            ctl("ab", "select", "a")
            # The match follows B's level over a 2 s window; give it time after the −12 dB step.
            matched_peak = record(6.0, "miceq_ab")
            ctl("ab", "match", "off")
            a_peak = record(1.0, "miceq_ab")
            assert 0.23 < b_peak < 0.27, f"B peak {b_peak}"
            assert 0.23 < matched_peak < 0.27, f"loudness-matched A peak {matched_peak}"
            assert 0.47 < a_peak < 0.53, f"A peak {a_peak}"
            waveform = json.loads(ctl("ab", "waveform", "--points", "64"))
            assert len(waveform["a"]) == 64 and 0.45 < max(waveform["a"]) <= 0.5, waveform
            assert 0.2 < max(waveform["b"]) < 0.27, waveform
            state = json.loads(ctl("ab", "stop", "--json"))
            assert not state["playing"], state
            smoke.wait_for(lambda: not ports("miceq_ab", "out"), "A/B stream removed")
            print(f"ab: B {b_peak:.3f} (live −12 dB: {b_live_peak:.3f}), A {a_peak:.3f}, "
                  f"A matched to B {matched_peak:.3f}; waveform and stop")

            ctl("preset", "broadcast")
            state = status()
            assert state["preset"] == "broadcast" and state["latency_samples"] == 960, state
            def reported_latency():
                for item in json.loads(run("pw-dump")):
                    if item.get("info", {}).get("props", {}).get("node.name") == "miceq_mic":
                        return item["info"].get("params", {}).get("ProcessLatency", [{}])[0].get("rate")
            smoke.wait_for(lambda: reported_latency() == 960, "ProcessLatency update in the graph")
            peak = record(1.0)
            assert 0.01 < peak <= 10 ** (-1 / 20), f"broadcast peak {peak}"
            # Nothing records the virtual microphone here: meters must run anyway.
            meters = [json.loads(line) for line in
                      ctl("meters", "--hz", "30", "--count", "5").splitlines()]
            assert len(meters) == 5 and meters[-1]["raw_peak_dbfs"] > -7, meters[-1]
            print(f"control: preset switched with crossfade, graph latency 960, peak {peak:.3f}; "
                  f"meters raw {meters[-1]['raw_peak_dbfs']:.1f} dBFS")

            with socket.socket(socket.AF_UNIX) as raw:
                raw.connect(str(runtime / "miceq" / "miceqd.sock"))
                stream = raw.makefile("rw")
                stream.write('{"id":1,"method":"mic.get"}\nnot json\n')
                stream.flush()
                before_hello = json.loads(stream.readline())
                garbage = json.loads(stream.readline())
                assert before_hello["id"] == 1 and before_hello["error"]["code"] == "bad_request"
                assert garbage["id"] is None and garbage["error"]["code"] == "bad_request"
                stream.write('{"id":2,"method":"hello","params":{"protocol":99,"client":"t"}}\n')
                stream.flush()
                mismatch = json.loads(stream.readline())
                assert mismatch["error"]["code"] == "protocol_mismatch", mismatch
            print("protocol: hello required, bad lines and protocol mismatch reported")

            stop(daemon)
            assert daemon.returncode == 0, f"daemon did not stop cleanly: {daemon.returncode}"
            saved = (config_home / "miceq" / "config.toml").read_text()
            assert 'preset = "broadcast"' in saved and 'device = "test_microphone"' in saved, saved
            assert not (runtime / "miceq" / "miceqd.sock").exists(), "socket left behind"
            daemon = start_daemon()
            state = status()
            assert state["preset"] == "broadcast" and state["device"] == "test_microphone", state
            assert record() > 0.01, "restored config produced no audio"

            # Calibration: 6.5 s of −60 dBFS noise, then gated "speech" at −24 dBFS RMS.
            # The capture starts within ~2.5 s of linking, so the silence step sees only noise.
            noise_gain, voice_gain = 10 ** (-60 / 20) * math.sqrt(3), 10 ** (-24 / 20)
            seed, samples = 12345, array.array("f")
            for n in range(int(48000 * 16.5)):
                seed = (seed * 1664525 + 1013904223) % 2 ** 32
                value = (seed / 2 ** 31 - 1) * noise_gain
                t = n / 48000
                if t >= 6.5 and (t * 3) % 1 < 0.5:
                    value += voice_gain * (math.sin(2 * math.pi * 200 * t)
                                           + math.sin(2 * math.pi * 400 * t))
                samples.append(value)
            if sys.byteorder != "little":
                samples.byteswap()
            (directory / "speech.raw").write_bytes(samples.tobytes())
            spawn(["pw-cat", "--playback", "--raw", "--format", "f32", "--channels", "1",
                   "--channel-map", "MONO", "--target", "0", "--latency", "128", "--properties",
                   "node.name=test_speaker media.class=Audio/Source adapter.auto-port-config={ mode=dsp position=preserve }",
                   directory / "speech.raw"])
            smoke.wait_for(lambda: ports("test_speaker", "out"), "speaker ports")
            ctl("device", "test_speaker")
            smoke.wait_for(lambda: status()["input_connected"], "speaker linked")
            # Nothing consumes the virtual microphone: calibration must still run.
            output = subprocess.check_output([str(args.ctl), "ctl", "calibrate", "--json"],
                                             env=env, text=True, timeout=20)
            report = json.loads(output)["report"]
            assert abs(report["noise_floor_dbfs"] + 60) < 1.5, report
            assert abs(report["speech_rms_dbfs"] + 24) < 1.5, report
            assert 5 <= report["gain_db"] <= 7, report
            state = status()
            assert state["settings"]["gain"]["gain_db"] == report["gain_db"], state
            assert abs(state["noise_floor_dbfs"] - report["noise_floor_dbfs"]) < 1e-3, state
            print(f"calibration: floor {report['noise_floor_dbfs']:.1f} dBFS, speech "
                  f"{report['speech_rms_dbfs']:.1f} dBFS, SNR {report['snr_db']:.1f} → "
                  f"{report['snr_after_db']:.1f} dB, gain {report['gain_db']:+.1f} dB applied")

            # User presets: calibration modified Broadcast; save it, edit, reload, share.
            assert status()["preset"] == "broadcast" and status()["preset_modified"]
            saved = json.loads(ctl("presets", "save", "Smoke voice", "--json"))
            assert saved == {"id": "user:smoke-voice", "name": "Smoke voice", "builtin": False}, saved
            preset_file = config_home / "miceq" / "presets" / "mic" / "smoke-voice.json"
            assert json.loads(preset_file.read_text())["name"] == "Smoke voice"
            state = status()
            assert state["preset"] == "user:smoke-voice" and not state["preset_modified"], state
            ctl("gain", "-3")
            assert status()["preset_modified"]
            ctl("presets", "update", "user:smoke-voice")
            assert not status()["preset_modified"]
            ctl("preset", "natural")
            ctl("preset", "user:smoke-voice")
            assert status()["settings"]["gain"]["gain_db"] == -3.0
            refused = subprocess.run([str(args.ctl), "ctl", "presets", "save", "natural"], env=env,
                                     capture_output=True, text=True, timeout=8)
            assert refused.returncode != 0 and "AlreadyExists" in refused.stderr, refused
            exported = directory / "shared.miceq-preset.json"
            ctl("presets", "export", "user:smoke-voice", exported)
            imported = json.loads(ctl("presets", "import", exported, "--json"))
            assert imported["name"] == "Smoke voice (2)", imported
            copy = json.loads(ctl("presets", "duplicate", "broadcast", "--json"))
            assert copy["name"] == "Broadcast (2)" and not copy["builtin"], copy
            ctl("presets", "rename", copy["id"], "Loud")
            ctl("presets", "delete", "user:smoke-voice")
            assert not preset_file.exists() and status()["preset"] is None
            names = [preset["name"] for preset in json.loads(ctl("presets", "--json"))]
            assert names[-2:] == ["Smoke voice (2)", "Loud"], names
            print("presets: save/update/select, duplicate name refused, export → import, "
                  "duplicate, rename, delete")

            # Fallback: distinct levels prove that the actual audio link follows priority.
            def microphone(name, amplitude):
                samples = array.array("f", [amplitude * math.sin(2 * math.pi * 440 * n / 48000)
                                            for n in range(48000 * 30)])
                if sys.byteorder != "little":
                    samples.byteswap()
                path = directory / f"{name}.raw"
                path.write_bytes(samples.tobytes())
                process = spawn(["pw-cat", "--playback", "--raw", "--format", "f32", "--channels", "1",
                                 "--channel-map", "MONO", "--target", "0", "--latency", "128",
                                 "--properties", f"node.name={name} media.class=Audio/Source "
                                 "adapter.auto-port-config={ mode=dsp position=preserve }", path])
                smoke.wait_for(lambda: ports(name, "out"), f"{name} ports")
                return process

            def active(name):
                state = status()
                return state["active_device"] == name and state["input_connected"] == (name is not None)

            primary = microphone("test_primary", 0.5)
            fallback = microphone("test_fallback", 0.2)
            ctl("preset", "raw")
            ctl("device", "test_primary")
            ctl("fallback-device", "test_fallback")
            smoke.wait_for(lambda: active("test_primary"), "primary selected")
            assert 0.45 < record() < 0.55, "primary priority failed"
            stop(primary)
            smoke.wait_for(lambda: active("test_fallback"), "fallback selected")
            assert status()["device"] == "test_primary", "fallback replaced the primary preference"
            assert 0.18 < record() < 0.22, "fallback audio missing"
            assert request("calibration.start")["state"] == "measuring"
            primary = microphone("test_primary", 0.5)
            smoke.wait_for(lambda: active("test_primary"), "primary restored")
            calibration = request("calibration.get")
            assert calibration["state"] == "failed" and calibration["reason"] == "disconnected", calibration
            assert 0.45 < record() < 0.55, "primary audio not restored"
            assert request("ab.record", {"max_seconds": 10})["recording"] is not None
            stop(primary)
            smoke.wait_for(lambda: active("test_fallback"), "fallback selected during recording")
            assert request("ab.get")["recording"] is None, "recording mixed two microphones"
            stop(fallback)
            smoke.wait_for(lambda: active(None), "both microphones disconnected")
            assert ports("miceq_mic", "out"), "virtual microphone disappeared"
            assert record() == 0.0, "disconnected microphone leaked audio"
            fallback = microphone("test_fallback", 0.2)
            smoke.wait_for(lambda: active("test_fallback"), "fallback reconnected")
            ctl("device", "none")
            smoke.wait_for(lambda: active(None), "explicit input disable")
            assert record() == 0.0, "fallback ignored explicit disable"
            ctl("device", "test_primary")
            smoke.wait_for(lambda: active("test_fallback"), "fallback reselected")
            stop(daemon)
            daemon = start_daemon()
            state = status()
            assert state["fallback_device"] == "test_fallback" and active("test_fallback"), state
            assert 0.18 < record() < 0.22, "saved fallback not restored"
            ctl("fallback-device", "none")
            smoke.wait_for(lambda: active(None), "fallback disabled")
            print("fallback: primary priority, failover audio, automatic return, silence, "
                  "explicit disable and config restored after restart")

            # Autostart through XDG (the user's systemd session is left alone).
            entry = config_home / "autostart" / "miceqd.desktop"
            assert json.loads(ctl("autostart", "--json")) == {"enabled": False, "method": "xdg_autostart"}
            assert json.loads(ctl("autostart", "on", "--json"))["enabled"]
            assert f"Exec={args.daemon.resolve()} run" in entry.read_text(), entry.read_text()
            assert not json.loads(ctl("autostart", "off", "--json"))["enabled"]
            assert not entry.exists()
            print("autostart: XDG entry written and removed")
            stop(daemon)
            stop(server)
            print("config: saved on change, restored on restart; socket removed on exit")
        except Exception:
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
