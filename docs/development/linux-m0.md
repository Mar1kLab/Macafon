# Linux M0: первый работающий аудиопуть

Состояние на 2026-10-09: начат M0 из PRD §5/§19. Это технический прототип без UI,
а не готовый MVP M1. Окончательный выбор схемы и ADR откладываются до завершения
проверок задержки, DeepFilterNet и совместимости версий PipeWire/WirePlumber.

## Что реализовано

- Cargo workspace, Rust stable, edition 2024. Созданы только используемые сейчас
  `miceq-dsp`, `miceq-backend-pipewire`, `miceqd`; остальные крейты появятся вместе с кодом.
- Один собственный `pw_filter`, `Audio/Source/Virtual`, `miceq_mic`, описание
  `micEQ Microphone` — основа MIC-010/MIC-011. Вход/выход mono float32, граф 48 кГц.
- Raw-путь с нулевой алгоритмической задержкой, цифровой mute (MIC-013),
  санитизация NaN/Inf/денормалов на границе цепочки (NFR-S4).
- RNNoise Lite через `nnnoiseless`, адаптер кадров 480 сэмплов к любому quantum,
  выравнивание dry/wet и сглаживание VAD-ослабления за 10 мс (основа NS-001/DSP-003).
  Диапазоны intensity/VAD и значения по умолчанию централизованы в `miceq-dsp`.
- Память выделяется до подключения фильтра. Callback целиком проверяется через
  `assert_no_alloc`, включая release-сборку. FTZ/DAZ включает PipeWire через
  свойство контекста `cpu.zero.denormals` (NFR-S1/NFR-S4).
- Привязка входа по `node.name`, явный mono/левый порт. Registry отслеживает
  исчезновение и возвращение портов: источник сохраняется, без входа выдаёт тишину,
  после возвращения того же имени линк создаётся снова (основа MIC-005).
- CLI-список устройств/потоков с `node.description`. Собственные узлы и `.monitor`
  исключаются (основа MIC-001/MIC-003; иконок и UI пока нет).
- `route` проверяет типы потока/приёмника, записывает `target.object` в `default`
  metadata по `object.serial` приёмника и подтверждает перенос по реальным линкам
  WirePlumber (основа APP-004). При таймауте восстанавливает прежний target, если другой
  клиент не успел его изменить. `node.dont-reconnect`/`node.dont-move` проверяются
  через полный NodeInfo: короткие registry properties этих флагов не содержат.
- После потери PipeWire демон ждёт сервер и пересоздаёт фильтр/линки/DSP вне RT,
  сохраняя выбранный вход и параметры CLI (основа NFR-S8). В ожидании работают
  SIGINT/SIGTERM; два последовательных перезапуска проверены интеграционно.
- Фиксированные DSP-порты сопровождаются node `EnumFormat` mono F32P/48 кГц
  и публикацией согласованного `Format`/`Props` на control loop.
  `pw_filter` не имеет node `PortConfig`; WirePlumber завершает согласование
  уже существующих портов после уведомления `Props` с mono channelMap. Это позволяет
  WirePlumber обнаружить источник и подключить обычного потребителя (MIC-010/MIC-011),
  не меняя порты или DSP-состояние в RT.
- Алгоритмическая задержка передаётся через SPA `ProcessLatency` (NFR-L4).
  SIGINT/SIGTERM обрабатываются до запуска data-loop threads; выход освобождает узел.

`miceqd run` ничего не устанавливает в systemd и не меняет системные default-устройства.
Роутинг изменяется только явной командой `route`; сохранения правил пока нет.
Один экземпляр фильтра соответствует одному микрофону; DSP-состояние не глобальное,
поэтому будущие независимые экземпляры цепочки не потребуют переделки DSP.

## Сборка и запуск

Нужны PipeWire ≥ 1.0, WirePlumber ≥ 0.5, Rust stable, C compiler, pkg-config,
libclang и заголовки PipeWire/SPA. Для Ubuntu:

```sh
sudo apt install build-essential pkg-config libclang-dev libpipewire-0.3-dev pipewire-bin
cargo build --release -p miceqd
target/release/miceqd devices
target/release/miceqd run --input 'alsa_input.<имя из devices>' --engine lite
```

Для реального времени используется release-сборка: debug-инференс и конкурирующая сборка
могут приводить к пропускам циклов и нестабильным замерам на headless-машине.
В приложении-потребителе нужно выбрать `micEQ Microphone`. Ctrl+C завершает прототип.
Без `--input` вход остаётся несвязанным; для экспериментов его можно соединить `pw-link`.
Если указанное устройство отсутствует, прототип ждёт появления этого имени.
Если сервер ещё не запущен или перезапущен, повторяет подключение раз в 500 мс.
Состояние RNNoise при восстановлении создаётся заново; краткий разрыв звука неизбежен.

```sh
target/release/miceqd run --input 'alsa_input.<name>' --engine raw
target/release/miceqd run --input 'alsa_input.<name>' --engine lite --intensity 70 --voice-threshold 50
target/release/miceqd run --input 'alsa_input.<name>' --mute
target/release/miceqd route <playback-stream-id> <sink-node.name>
```

`devices` выводит ID, media.class, node.name, человекочитаемое описание.
`route` ждёт до трёх секунд подтверждения фактического переноса по активным/готовым
линкам. Ошибка политики не считается успехом записи metadata. WirePlumber должен
разрешать `linking.allow-moving-streams`; потоки с `node.dont-reconnect=true` или
`node.dont-move=true` команда отклоняет, не меняя их target. Это подтверждённое
ограничение подхода APP-004, а не закрытое требование «переносить любые приложения».
Автоматического перехвата приложений и persistence пока нет.

## Локальная проверка

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p miceqd
python3 tools/tests/pipewire-smoke.py
python3 tools/tests/wireplumber-routing.py
cargo bench -p miceq-dsp --bench microphone -- --sample-size 10 --measurement-time 1 --warm-up-time 1
```

Smoke-тест использует отдельный PipeWire с приватным runtime-каталогом и синтетическим
микрофоном. Пользовательский аудиосервер и физические устройства не затрагиваются.
Нужны `pipewire`, `pw-cat`, `pw-link`, `pw-dump`, `pw-metadata` из пакета PipeWire tools.
Тест сам завершает дочерние процессы и удаляет временный runtime-каталог.
Снимки синтетического сигнала остаются в `.artifacts/latency/`, исключённой из git.

Проверяются прохождение сигнала, mute, тишина после исчезновения входа, переподключение
по имени, удаление источника при SIGTERM, два восстановления после потери сервера
с сохранением Lite/задержки и прохождением сигнала, ожидание отсутствующего сервера
с выходом по SIGINT, ошибка роутинга без policy manager и откат metadata.
DSP-тест отдельно запускает первый блок на новом потоке под `assert_no_alloc`,
проверяет произвольный размер блоков, выравнивание dry-пути, NaN/Inf и bit-exact raw.

Отдельный `wireplumber-routing.py` требует WirePlumber, `wpctl` и `dbus-run-session`.
Он создаёт приватные D-Bus/runtime/config/state, запускает только policy-профиль
без аппаратных мониторов и не затрагивает пользовательский WirePlumber. Проверяет
Main → Voice → Main → Voice без пересоздания потока, отказ при выключенном
`linking.allow-moving-streams` с восстановлением прежнего target, явный отказ
для `node.dont-reconnect=true`/`node.dont-move=true`, а также подключение
потребителя к виртуальному микрофону через WirePlumber и прохождение аудио.

`tools/latency-probe/analyze.py` сравнивает синхронные каналы stereo float32 LE raw:
FL — прямой путь синтетического импульса, FR — через micEQ. Нестабильная задержка
считается ошибкой, а не усредняется. Пример:

```sh
python3 tools/latency-probe/analyze.py .artifacts/latency/raw.raw
```

## Результат M0 и открытые проблемы

На PipeWire 1.6.2, 48 кГц, quantum 128, release:

| Путь | Алгоритмическая задержка | Измеренная добавленная задержка |
|---|---|---|
| Raw | 0 сэмплов | 128 сэмплов / 2,67 мс — NFR-L1 достигнут в headless-графе |
| Lite, intensity 0 | 960 сэмплов / 20 мс | 1088 сэмплов / 22,67 мс — NFR-L2 **не достигнут** |

Intensity 0 оставляет RNNoise включённым и исполняющимся, но использует выровненный
dry-сигнал для точного импульсного замера. Для полного обхода без задержки используется
`--engine raw`. Этот замер не заменяет проверку wet-пути на речи и физического loopback.

PRD §10.1 считает задержку Lite равной 10 мс. `nnnoiseless` использует окно overlap-add
из двух кадров; streaming-адаптер дополнительно собирает кадр. В текущем прототипе
получаются два кадра, 20 мс, плюс один quantum графа. Целевое значение PRD не менялось:
нужно исследовать сокращение адаптерной задержки/схему графа и затем согласовать решение.

`nnnoiseless` закреплён на **0.5.1**: в 0.5.2 FFT перешёл на `easyfft` с ленивыми
thread-local планировщиками/буферами. Подготовка в control thread не устраняет первую
аллокацию на другом RT-потоке. Версия 0.5.1 создаёт FFT и scratch при конструировании
и проходит проверку первого блока на новом потоке. Лицензия BSD-3-Clause совместима
с вариантами лицензии из PRD Q1; лицензию самого проекта этот раунд не выбирает.

Остаются до завершения M0:

- DeepFilterNet/Studio: модель, проверка RT-безопасности и бюджета CPU/задержки (NS-002).
- Матрица версий PipeWire/WirePlumber и реальные native/Flatpak/Electron приложения.
  На PipeWire 1.6.2 / WirePlumber 0.5.13 обычный native-поток переносится;
  `node.dont-reconnect=true` блокирует перенос. Нужен дальнейший выбор подхода
  для таких потоков без нарушения политики/настроек пользователя.
- Suspend/resume и физическое переподключение устройств (NFR-S8/MIC-005):
  синтетический hotplug и перезапуск сервера проверены, аппаратные сценарии — ещё нет.
- Физический loopback и часовой soak с учётом xruns, проверка CPU/RAM на эталонном ПК.
- Поддержка графов 44,1/96 кГц через PipeWire adapter (DSP-006): сейчас несовпадение частоты
  приводит к тишине и диагностическому завершению, а не к обработке на неверной частоте.

UI, восемь DSP-модулей, изменение параметров во время работы, IPC, конфиг/пресеты,
калибровка/A-B, трей, автозапуск и упаковка относятся к следующему этапу M1.
