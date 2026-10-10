# M1: UI (Tauri v2 + Svelte 5)

Состояние на 2026-10-10. Готов экран Microphone, калибровка, панель TEST (A/B) и базовые Settings.
Онбординг, импорт/экспорт пресетов через файловый диалог и экран Mixer — следующие шаги.

## Запуск

```sh
pnpm -C apps/macafon-ui install
pnpm -C apps/macafon-ui dev            # браузер, mock-демон: http://127.0.0.1:1420
pnpm -C apps/macafon-ui tauri dev      # окно Tauri с настоящим демоном
pnpm -C apps/macafon-ui tauri build --no-bundle   # target/release/macafon-ui
```

`macafon` без аргументов запускает `macafon-ui` (рядом с бинарём или из `PATH`) — так его открывают
трей (BG-004) и ярлык. Второй запуск фокусирует уже открытое окно (BG-006, `tauri-plugin-single-instance`).
Закрытие окна завершает процесс UI, звук остаётся в демоне (BG-001).

На этой машине нет системных WebKitGTK/GTK: заголовки и `.so` лежат в том же user-local sysroot,
что и PipeWire (`. ~/.cache/macafon-sysroot/env.sh`). Сборка и тесты работают; запустить окно здесь
нельзя — WebKit ищет свои процессы по зашитому пути `/usr/lib/.../webkit2gtk-4.1`, а user namespaces
для подмены пути запрещены. UI проверяется в браузере на mock-демоне, мост — тестами (ниже).

## Устройство

```
Svelte (runes) ──DaemonClient──► Transport ─┬─ TauriTransport ─invoke/event─► src-tauri/bridge.rs ─► macafond.sock
                                            └─ MockTransport (браузер; ?daemon=down — демон недоступен,
                                                              ?mic=disconnected — микрофон отключён)
```

- `src/lib/ipc/protocol.ts` и `dsp.ts` **генерируются** из Rust (`ts-rs` за фичей `ts`, по умолчанию
  выключена — демон её не тянет): типы протокола, диапазоны и значения по умолчанию из `params.rs`,
  кривые макросов, характеры EQ и встроенные пресеты. Руками не править:
  `pnpm -C apps/macafon-ui gen:ipc`. `cargo test -p macafon-ipc --features ts` падает, если файлы устарели.
- `client.ts`: типизированный `call(method, params)`; таблица результатов проверяется на полноту
  против сгенерированного `Request` — новый метод протокола не скомпилируется, пока его не добавят.
- `state/daemon.svelte.ts`: зеркало состояния демона. Правка цепочки — оптимистично локально, затем
  `mic.set`; пока запрос в полёте, новые правки копятся и отправляется только последняя (не больше
  одного запроса одновременно). Эхо `mic.changed` во время своих правок не перетирает слайдер.
  Undo/Redo (DSP-008): правки ближе 600 мс — один шаг, глубина 100, mute/bypass не откатываются.
- Метры (MTR-004, NFR-R7): подписка 60 Гц только пока окно видимо (`visibilitychange`). Кадры лежат
  вне реактивности (`state/meters.ts`), canvas-метры рисуются в одном общем `requestAnimationFrame`.
  Баллистика из дизайн-системы: peak-hold 1 с, спад 20 дБ/с, клип «залипает» на 2 с.
- `bridge.rs`: один поток владеет соединением — handshake `hello`, ответы сопоставляются по `id`,
  события и статус уходят в WebView (`daemon://event`, `daemon://status`). Если сокета нет —
  один раз запускает `macafond run` (рядом с бинарём или из `PATH`, отдельная группа процессов, чтобы
  демон пережил UI) и ждёт 4 с (§9.7). Потерянное соединение не перезапускает демон (после Quit
  из трея он должен остаться выключенным; упавший поднимет systemd) — через 4 с экран «offline»
  с Retry. `Quit Macafon` в Settings останавливает демон и закрывает UI (BG-005).

## Понятность для новичка

- Блок «Getting started» на экране Microphone: калибровка (отмечается выполненной), запись и
  сравнение, выбор «Macafon Microphone» в приложениях + отключить их шумодав (US-2/3/5, MIC-015).
  Скрывается кнопкой «Got it» (флаг в `localStorage` WebView).
- Пока калибровки не было, кнопка Calibrate — главная (инвертированная). «Listen A/B» в итоге
  калибровки сразу запускает запись в панели TEST.
- Баннеры состояния: микрофон не выбран, отключён (MIC-005), обработка выключена (Bypass) с
  кнопкой «Turn on», клип на входе.
- Видимые подписи без жаргона: Before/After вместо Raw/Processed, Match volume, Cut и Noise вместо
  GR/NS, Mic вместо RAW; у метров, Bypass, Mute, Latency — однострочные подсказки при наведении.
  Индикатор CLIP появляется только при клипе.
- Save в шапке виден только когда пресет изменён. Mixer скрыт из навигации до M2.
- Quit в Settings — с подтверждением.

## Макросы и CUSTOM (DSP-002)

В `ChainSettings` нет положения главного слайдера — UI выводит его из параметров, обращая кривые
`macafon_dsp::settings::macros` (они экспортируются в `dsp.ts`). Если параметры не совпадают ни с
одним положением слайдера — модуль `CUSTOM`, слайдер заблокирован, есть «Reset to simple».

| Модуль | Главный слайдер | Как определяется |
|---|---|---|
| Gain, High-pass, NS, Limiter | Сам параметр (gain, частота, intensity, ceiling) | Всегда простой режим |
| Expander | Sensitivity | По ratio; range и времена должны совпасть. **Порог не сравнивается**: калибровка ставит его по шумовому полу (CAL-003), и модуль не должен из-за этого становиться CUSTOM. Движение слайдера ставит порог от пола после Gain (пол из калибровки + gain) или от −56 dBFS без калибровки |
| Compressor | Amount | По порогу; ratio, attack, release, knee, auto makeup, RMS должны совпасть |
| De-esser | Intensity | По порогу; частота, max reduction, split-band. Listen не учитывается |
| EQ | Character + Amount | 5 полос раскладки по умолчанию с усилениями характера × Amount; любая правка полос — CUSTOM |

`macros.test.ts` проверяет обратное отображение на встроенных пресетах из Rust — если TS-зеркало
разойдётся с `from_amount`/`from_sensitivity`, тест упадёт.

## Отклонения и пробелы

- MIC-002: живой мини-метр есть только у активного микрофона — демон метрит лишь подключённый вход.
  Для метров всех устройств в списке нужен захват каждого входа в `macafond` на время открытого списка.
- MIC-001: тип устройства (USB/гарнитура/Bluetooth/встроенный) угадывается по `node.name`;
  `devices.list` не отдаёт `device.bus`/`device.form-factor`.
- Импорт/экспорт пресетов (P1) в UI нет — нужен файловый диалог (`tauri-plugin-dialog`); в CLI есть.
- BG-001: тост «Macafon keeps running in the tray» при первом закрытии не сделан (процесс UI к этому
  моменту завершается — нужен показ до закрытия или уведомление от демона).
- NS Studio выбирается, но демон работает на Lite (DeepFilterNet ещё не встроен) — UI это пишет.

## Проверка

```sh
pnpm -C apps/macafon-ui check                       # svelte-check + TypeScript strict
pnpm -C apps/macafon-ui test                        # Vitest: макросы против пресетов, АЧХ EQ
cargo test -p macafon-ipc --features ts             # TS-файлы актуальны
cargo test -p macafon-ui                            # мост против фейкового демона
pnpm -C apps/macafon-ui screenshots ../../.artifacts/screenshots [сценарий…]   # при запущенном dev
```

Тест моста поднимает фейковый демон на Unix-сокете: ответы в другом порядке, событие между ними,
ошибка демона, пропажа демона с незавершённым вызовом, переход в `unavailable` без запуска второго
демона. Разово мост проверен и против настоящего `macafond` в приватном PipeWire: handshake, вызовы,
60 кадров метров за 2 с при 30 Гц, маппинг ошибок, `daemon.quit` → `unavailable`.
