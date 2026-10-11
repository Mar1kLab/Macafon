#!/usr/bin/env python3
"""Measure daemon follower xruns in a private 48 kHz/128-frame PipeWire graph."""
import argparse
import array
import json
import math
import os
from pathlib import Path
import subprocess
import threading
import time
from m1_private import Client, PrivateGraph, ROOT, smoke


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=3600)
    parser.add_argument("--preset", default="natural")
    parser.add_argument("--report", type=Path, default=ROOT / ".artifacts/soak/microphone.json")
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error("--seconds must be positive")
    # The helper creates only this test's private daemon, never the user's server.
    smoke.CONFIG = smoke.CONFIG.replace("{ name = libpipewire-module-protocol-native }", "{ name = libpipewire-module-protocol-native }\n    { name = libpipewire-module-profiler }")
    stats = {"observations": 0, "xrun_baseline": None, "xrun_max": 0, "max_process_ms": 0.0,
             "max_wake_delay_ms": 0.0, "max_deadline_lateness_ms": 0.0,
             "over_budget_observations": 0}
    errors = []
    stop_feed = threading.Event()
    with PrivateGraph() as graph:
        daemon = graph.daemon()
        client = Client(graph.socket_path)
        client.request("mic.set_preset", {"preset": args.preset})
        source = subprocess.Popen(["pw-cat", "--playback", "--raw", "--format", "f32",
            "--channels", "1", "--channel-map", "MONO", "--rate", "48000", "--latency", "128",
            "--target", "0", "--properties",
            "node.name=test_microphone media.class=Audio/Source adapter.auto-port-config={ mode=dsp position=preserve }", "-"],
            env=graph.env, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        graph.processes.append(source)
        tone = array.array("f", [0.12 * math.sin(2 * math.pi * 440 * i / 48000) for i in range(4800)]).tobytes()

        def feed():
            try:
                while not stop_feed.is_set():
                    source.stdin.write(tone)
                    source.stdin.flush()
            except (BrokenPipeError, OSError):
                pass

        feeder = threading.Thread(target=feed, daemon=True)
        feeder.start()
        consumer = graph.spawn(["pw-cat", "--record", "--raw", "--format", "f32", "--channels", "1",
            "--channel-map", "MONO", "--latency", "128", "--target", "0", "--properties",
            "node.name=test_soak_capture adapter.auto-port-config={ mode=dsp position=preserve }", "/dev/null"])
        def ports(name, direction):
            objects = json.loads(graph.run("pw-dump"))
            nodes = {item["id"] for item in objects if item.get("info", {}).get("props", {}).get("node.name") == name}
            return [item["id"] for item in objects if item.get("info", {}).get("props", {}).get("port.direction") == direction
                    and int(item.get("info", {}).get("props", {}).get("node.id", -1)) in nodes]
        capture = smoke.wait_for(lambda: ports("test_soak_capture", "in"), "soak consumer")
        output = smoke.wait_for(lambda: ports("macafon_mic", "out"), "soak microphone")
        graph.run("pw-link", str(output[0]), str(capture[0]))
        smoke.wait_for(lambda: client.request("mic.get")["input_connected"], "soak physical input")
        time.sleep(2)
        profiler = subprocess.Popen(["pw-profiler", "--json"], env=graph.env, stdout=subprocess.PIPE,
                                    stderr=subprocess.DEVNULL, text=True, bufsize=1)
        graph.processes.append(profiler)

        def monitor():
            for line in profiler.stdout:
                line = line.strip().lstrip("[").rstrip(",")
                if not line.startswith("{"):
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if row.get("type") == "follower" and row.get("name") == "macafon_mic":
                    stats["observations"] += 1
                    count = row["xrun_count"]
                    if stats["xrun_baseline"] is None:
                        stats["xrun_baseline"] = count
                    stats["xrun_max"] = max(stats["xrun_max"], count)
                    elapsed_ms = max(0, row["finish"] - row["awake"]) / 1_000_000
                    stats["max_process_ms"] = max(stats["max_process_ms"], elapsed_ms)
                    # Processing duration excludes the delay before the follower gets CPU.
                    # Both are useful diagnostics; neither alone attributes an xrun's cause.
                    if row["finish"] >= row["awake"] > 0 and row["signal"] > 0:
                        budget_ms = 128 / 48000 * 1000
                        wake_delay_ms = max(0, row["awake"] - row["signal"]) / 1_000_000
                        deadline_lateness_ms = max(0, (row["finish"] - row["signal"]) / 1_000_000 - budget_ms)
                        stats["max_wake_delay_ms"] = max(stats["max_wake_delay_ms"], wake_delay_ms)
                        stats["max_deadline_lateness_ms"] = max(stats["max_deadline_lateness_ms"], deadline_lateness_ms)
                        if elapsed_ms > 0.8 * budget_ms:
                            stats["over_budget_observations"] += 1

        observer = threading.Thread(target=monitor, daemon=True)
        observer.start()
        def resources():
            status = Path(f"/proc/{daemon.pid}/status").read_text()
            rss_kib = next(int(line.split()[1]) for line in status.splitlines() if line.startswith("VmRSS:"))
            # Fields after comm: state is field 3, utime/stime are fields 14/15.
            fields = Path(f"/proc/{daemon.pid}/stat").read_text().rsplit(")", 1)[1].split()
            return rss_kib, int(fields[11]) + int(fields[12])
        rss_start, cpu_start = resources()
        stats["rss_kib_start"] = rss_start
        stats["rss_kib_max"] = rss_start
        started = time.monotonic()
        next_progress = 0.0
        while time.monotonic() - started < args.seconds:
            elapsed = time.monotonic() - started
            for name, process in (("daemon", daemon), ("source", source), ("consumer", consumer), ("profiler", profiler)):
                if process.poll() is not None:
                    errors.append(f"{name} exited: {process.returncode}")
            if stats["xrun_baseline"] is not None and stats["xrun_max"] > stats["xrun_baseline"]:
                errors.append("Macafon follower reported an xrun; stopped before the requested duration")
            if errors:
                break
            stats["rss_kib_max"] = max(stats["rss_kib_max"], resources()[0])
            if elapsed >= next_progress:
                print(f"soak {elapsed:.0f}/{args.seconds:.0f}s, samples {stats['observations']}, xruns {stats['xrun_max']}", flush=True)
                next_progress += 60
            time.sleep(min(1, max(0, args.seconds - elapsed)))
        actual = time.monotonic() - started
        rss_end, cpu_end = resources()
        stats["rss_kib_end"] = rss_end
        stats["cpu_one_core_pct"] = 100 * (cpu_end - cpu_start) / os.sysconf("SC_CLK_TCK") / actual
        graph.stop(profiler)
        observer.join(timeout=5)
        stop_feed.set()
        graph.stop(source)
        feeder.join(timeout=5)
        try:
            source.stdin.close()
        except BrokenPipeError:
            pass
        profiler.stdout.close()
        state = client.request("mic.get")
        client.close()
        stats.update(seconds=actual, requested_seconds=args.seconds, preset=args.preset,
                     effective_noise_engine=state["settings"]["noise_suppression"]["engine"],
                     quantum_frames=128, sample_rate_hz=48000, errors=errors)
        stats["xruns_during_test"] = stats["xrun_max"] - (stats["xrun_baseline"] or 0)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(stats, indent=2) + "\n")
        print(json.dumps(stats, indent=2))
        assert not errors, errors
        assert stats["observations"] > 0, "profiler did not observe the Macafon follower"
        assert stats["xruns_during_test"] == 0, stats


if __name__ == "__main__":
    main()
