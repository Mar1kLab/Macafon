#!/usr/bin/env python3
"""Verify native profiler delivery in a private, actively processing microphone graph."""
import array
import json
import math
import subprocess
from m1_private import Client, PrivateGraph, ROOT, smoke


def main():
    # Compile before starting audio, so compilation cannot cause test graph xruns.
    build = subprocess.run(["cargo", "test", "-p", "macafon-backend-pipewire", "--no-run",
                            "--message-format=json"], cwd=ROOT, text=True,
                           stdout=subprocess.PIPE, check=True)
    executable = next(row["executable"] for line in build.stdout.splitlines()
                      if (row := json.loads(line)).get("reason") == "compiler-artifact"
                      and row.get("executable") and row["target"]["name"] == "macafon_backend_pipewire")
    smoke.CONFIG = smoke.CONFIG.replace("{ name = libpipewire-module-protocol-native }",
        "{ name = libpipewire-module-protocol-native }\n    { name = libpipewire-module-profiler }")
    with PrivateGraph() as graph:
        graph.daemon()
        tone = graph.directory / "tone.raw"
        tone.write_bytes(array.array("f", (0.12 * math.sin(2 * math.pi * 440 * i / 48000)
                                          for i in range(48000 * 20))).tobytes())
        graph.spawn(["pw-cat", "--playback", "--raw", "--format", "f32", "--channels", "1",
            "--channel-map", "MONO", "--rate", "48000", "--latency", "128", "--target", "0",
            "--properties", "node.name=test_microphone media.class=Audio/Source adapter.auto-port-config={ mode=dsp position=preserve }", tone])
        client = Client(graph.socket_path)
        smoke.wait_for(lambda: client.request("mic.get")["input_connected"], "physical input")
        # Capture keeps the microphone processing even without a recording application.
        client.request("ab.record", {"max_seconds": 10})
        subprocess.run([executable, "--ignored", "--exact",
            "profiler::tests::receives_native_cycle_measurements_on_control_loop"], cwd=ROOT,
            env=dict(graph.env, MACAFON_TEST_PROFILER_NODE="macafon_mic"), check=True, timeout=8)
        client.close()
    print("profiler: native own-node durations and quantum reach the control loop")


if __name__ == "__main__":
    main()
