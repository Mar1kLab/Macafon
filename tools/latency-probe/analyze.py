#!/usr/bin/env python3
"""Compare synchronous direct/processed channels in a stereo float32 raw capture."""

import argparse
import array
import json
import math
import sys
from pathlib import Path


def analyze(path: Path, max_delay_samples: int = 2048) -> dict:
    samples = array.array("f")
    samples.frombytes(path.read_bytes())
    if sys.byteorder != "little":
        samples.byteswap()
    if len(samples) % 2:
        raise ValueError("capture must contain complete stereo float32 frames")
    direct, processed = samples[0::2], samples[1::2]
    if not all(math.isfinite(value) for value in samples):
        raise ValueError("non-finite audio in capture")
    # The probe emits isolated impulses. Ignore startup; look for the first stable pulse
    # and compare the corresponding peak in the other channel of the same capture.
    pulses = [i for i in range(4800, len(direct) - max_delay_samples)
              if abs(direct[i]) > 0.2]
    if not pulses:
        raise ValueError("no reference pulse; check direct-path wiring")
    delays = []
    for pulse in pulses:
        stop = pulse + max_delay_samples + 1
        peak = max(range(pulse, stop), key=lambda i: abs(processed[i]))
        if abs(processed[peak]) >= 0.1:
            delays.append(peak - pulse)
    if not delays:
        raise ValueError("no processed pulse; disable suppression or use Lite intensity 0")
    if min(delays) != max(delays):
        raise ValueError(f"unstable delay {min(delays)}..{max(delays)} samples; check xruns and repeat")
    delay = delays[0]
    return {"sample_rate_hz": 48000, "added_latency_samples": delay,
            "added_latency_ms": delay / 48.0, "matched_pulses": len(delays)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    args = parser.parse_args()
    print(json.dumps(analyze(args.capture), indent=2))
