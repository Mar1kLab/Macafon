# M1: UI (Tauri v2 + Svelte 5)

Состояние на 2026-10-12. Готовы онбординг, экраны Microphone и Mixer, калибровка, панель TEST (A/B)
и Settings. Импорт/экспорт пресетов через файловый диалог — следующий шаг. Микшер — [mixer.md](mixer.md).

Mock-транспорт (`src/lib/ipc/mock.ts`) управляет подключением, микрофоном, пресетами,
калибровкой и A/B. В `src/lib/ipc/mock/` вынесены данные устройств и приложений (`fixtures`),
чистые функции (`helpers`), синтетические сигналы (`signals`) и отдельный микшер (`mixer`).
Микшер владеет каналами, приложениями и метрами, использует сгенерированные IPC-типы
и отдаёт копии состояния для ответов и событий.

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
                                                              ?mic=disconnected — микрофон отключён,
                                                              ?config=recovered — конфиг сброшен,
                                                              ?overload — Studio → Lite)
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

## Онбординг (§9.2)

Полноэкранный поток вместо основного окна, пока в `localStorage` WebView нет `macafon.onboarding=done`.
Шесть шагов: Welcome → Microphone → Calibrate → Result → Use it → Background; «Skip setup» в шапке
завершает поток без побочных эффектов, Settings → Setup guide запускает его снова.

- Microphone: живые метры всех входов (`devices.subscribe` только пока шаг открыт), список обновляется
  раз в 2 с. Пока пользователь не выбрал сам, выбирается вход, дольше всех слышащий речь
  (выше −45 dBFS, 0,5 с); `mic.set_device` — только по Next.
- Calibrate/Result: те же `CalibrationProgress` / `CalibrationResult`, что в диалоге. Result встраивает
  панель TEST; «Listen A/B» записывает клип, «Fine-tune» проводит через шаги 5–6 и после Done
  открывает экран Microphone.
  Без калибровки шаг Result пропускается.
- Use it: переключатель системного default (`mic.set_default_source` с `confirmed: true` / `restore`),
  подсказки для Discord, OBS, Zoom (MIC-015).
- Background: «Start with system» включён по умолчанию и применяется только по Done (AS-001).

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
- Save в шапке виден только когда пресет изменён.

## Экран Mixer (§9.4)

- Колонка канала: полоса цвета, чипы приложений, вертикальный фейдер (стрелки ±1%, PageUp/Down ±10%,
  двойной клик — 100%) со стерео-метром, % и dB, `M` / `EQ●` / `FX●`. Боковая панель: EQ с пресетами
  по категориям, FX (Bypass канала, NS, Compressor Amount, Limiter Ceiling), настройки канала.
- Перенос приложения: HTML5 drag & drop (фишка с наклоном 2°, цель подсвечивается цветом и тиками),
  клавиши 1…8 на сфокусированной фишке, контекстное меню (Move to, Don't manage, Forget).
- Правки канала оптимистичны и сливаются (как у микрофона): одновременно не больше одного `channel.set`
  на канал. Метры `mixer.meters` идут вместе с `meters` по той же подписке.
- Mock: `?screen=mixer`, `?mixer=off` — микшер ещё не включён.
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

- Импорт/экспорт пресетов (P1) в UI нет — нужен файловый диалог (`tauri-plugin-dialog`); в CLI есть.
- BG-001: подсказку при первом закрытии показывает демон системным уведомлением (см. mixer.md).

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
