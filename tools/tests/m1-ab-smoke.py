#!/usr/bin/env python3
"""M1 A/B WAV export, output metadata and UI-close privacy on a private graph."""
import array
import math
import struct
from m1_private import Client, PrivateGraph, smoke


def main():
    with PrivateGraph() as graph:
        graph.daemon()
        audio = array.array("f", (0.1 * math.sin(2 * math.pi * 440 * i / 48000) for i in range(48000 * 20)))
        source = graph.directory / "speech.raw"
        source.write_bytes(audio.tobytes())
        graph.spawn(["pw-cat", "--playback", "--raw", "--format", "f32", "--rate", "48000",
                     "--channels", "1", "--channel-map", "MONO", "--target", "0", "--latency", "128",
                     "--properties", "node.name=test_microphone media.class=Audio/Source adapter.auto-port-config={ mode=dsp position=preserve }", source])
        ui = Client(graph.socket_path, "macafon-ui")
        smoke.wait_for(lambda: ui.request("mic.get")["input_connected"], "microphone linked")
        outputs = ui.request("ab.outputs")
        assert any(output["name"] == "test_sink" for output in outputs), outputs
        assert all("kind" in output and "speaker_warning" in output for output in outputs), outputs
        speaker = next(output for output in outputs if output["name"] == "test_sink")
        assert speaker["kind"] in ("speakers", "unknown") and speaker["speaker_warning"], speaker
        ui.request("ab.record", {"max_seconds": 3})
        smoke.wait_for(lambda: ui.request("ab.get")["clip_ms"] is not None, "three-second clip")
        settings = ui.request("mic.get")["settings"]
        settings["gain"]["enabled"] = True
        settings["gain"]["gain_db"] = 6.0
        ui.request("mic.set", {"settings": settings})
        samples = {}
        for side in ("a", "b"):
            path = graph.directory / f"{side}.wav"
            ui.request("ab.export", {"side": side, "path": str(path)})
            data = path.read_bytes()
            assert data[:4] == b"RIFF" and data[8:12] == b"WAVE"
            assert struct.unpack_from("<HHI", data, 20) == (3, 1, 48000)
            assert len(data) == 56 + 3 * 48000 * 4
            values = struct.unpack_from(f"<{3 * 48000}f", data, 56)
            assert max(map(abs, values)) > 0.01
            samples[side] = values
            ui.request("ab.export", {"side": side, "path": str(path)}, error=True)
        expected_gain = 10 ** (6 / 20)
        assert abs(max(map(abs, samples["b"])) / max(map(abs, samples["a"])) - expected_gain) < 0.01, "B export ignored current chain"
        cli = Client(graph.socket_path)
        cli.close()
        assert ui.request("ab.get")["clip_ms"] is not None, "CLI disconnected and erased UI clip"
        ui.request("ab.play", {"device": "test_sink"})
        assert ui.request("ab.get")["playing"]
        ui.close()
        observer = Client(graph.socket_path)
        smoke.wait_for(lambda: observer.request("ab.get")["clip_ms"] is None, "UI-close clears clip")
        assert not observer.request("ab.get")["playing"]
        assert observer.request("mic.get")["input_connected"], "UI-close interrupted live mic"
        observer.request("ab.export", {"side": "a", "path": str(graph.directory / "cleared.wav")}, error=True)
        observer.close()
        print("A/B: float WAV A/B, no overwrite, output warning metadata, CLI preservation and UI-close privacy passed")


if __name__ == "__main__":
    main()
