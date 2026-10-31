"""Regression: recorder sample loss must not be reported as DSP latency jitter."""
import array
import tempfile
import unittest
from pathlib import Path

from analyze import CaptureDiscontinuity, analyze


class CaptureTimingTest(unittest.TestCase):
    def test_sample_loss_is_retryable_but_dsp_jitter_is_not(self):
        for lost_samples in (0, 128):
            with self.subTest(lost_samples=lost_samples), tempfile.TemporaryDirectory() as directory:
                samples = array.array("f", [0.0]) * (80000 * 2)
                for index, pulse in enumerate((12000, 36000, 60000)):
                    samples[pulse * 2] = 0.5
                    samples[(pulse + 1088 + (128 if index == 1 and not lost_samples else 0)) * 2 + 1] = 0.5
                if lost_samples:
                    del samples[36500 * 2:(36500 + lost_samples) * 2]
                path = Path(directory) / "capture.raw"
                path.write_bytes(samples.tobytes())
                expected = CaptureDiscontinuity if lost_samples else ValueError
                with self.assertRaises(expected) as caught:
                    analyze(path, reference_period_samples=24000)
                self.assertEqual(type(caught.exception), expected)


if __name__ == "__main__":
    unittest.main()
