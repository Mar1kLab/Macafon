# Звуковые фикстуры

`voice-*.wav`: настоящие записи слов «yes» и «no» из Speech Commands (Pete Warden,
Google и участники датасета), с процедурными моделями шума вентилятора, клавиатуры
и улицы. Исходные записи — mono PCM16 16 кГц, тестовые — линейно пересэмплированы
в 48 кГц, громкость речи уменьшена вдвое, добавлена секунда паузы и шум.

Источник: [TensorFlow Lite Micro, testdata](https://github.com/tensorflow/tflite-micro/tree/main/tensorflow/lite/micro/examples/micro_speech/testdata),
`yes_1000ms.wav`, `no_1000ms.wav`. Датасет распространяется по
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/); атрибуция и изменения
описаны выше. Автор исходного датасета: Pete Warden, Google, Speech Commands, 2018.

SHA-256 исходников:

- yes: `b40e137a9d25bbdf135e03fa0f6bff4696d0bfc203aef8bc73b27c9428ba5a92`
- no: `21e2b640385e4a751d480a87ae5a1633a6ff00e838b0e2ad48b22f8034e946d8`

Файлы `.f32` — выходы текущей цепочки в каждой 97-й точке (little-endian float32).
Допуск — 0,0001 по амплитуде. Это регрессионная проверка формы сигнала, а не
доказательство субъективного качества речи. Обновление только явно:

```sh
MACAFON_UPDATE_GOLDEN=1 cargo test -p macafon-engine --test golden
```
