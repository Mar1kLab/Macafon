# Audio fixtures

`voice-*.wav`: real recordings of the words "yes" and "no" from Speech Commands (Pete Warden,
Google and the dataset contributors), with procedural models of fan, keyboard and street
noise. The source recordings are mono PCM16 at 16 kHz; the test files are linearly resampled
to 48 kHz, with speech volume halved, one second of silence and noise added.

Source: [TensorFlow Lite Micro, testdata](https://github.com/tensorflow/tflite-micro/tree/main/tensorflow/lite/micro/examples/micro_speech/testdata),
`yes_1000ms.wav`, `no_1000ms.wav`. The dataset is distributed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/); attribution and changes are
described above. Author of the original dataset: Pete Warden, Google, Speech Commands, 2018.

SHA-256 of the sources:

- yes: `b40e137a9d25bbdf135e03fa0f6bff4696d0bfc203aef8bc73b27c9428ba5a92`
- no: `21e2b640385e4a751d480a87ae5a1633a6ff00e838b0e2ad48b22f8034e946d8`

The `.f32` files are the current chain's outputs at every 97th point (little-endian float32).
The tolerance is 0.0001 in amplitude. This is a regression check of the waveform, not proof of
subjective speech quality. Update only explicitly:

```sh
MACAFON_UPDATE_GOLDEN=1 cargo test -p macafon-engine --test golden
```
