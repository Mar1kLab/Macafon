# M1: демон, управление на лету, конфиг и IPC

Состояние на 2026-10-09. Демон `miceqd` работает постоянно, им можно управлять во время работы
через Unix-сокет; настройки сохраняются. UI ещё нет — его роль пока выполняет `miceq ctl`.

## Устройство

```
IPC-потоки (по 2 на клиента) ──pipewire::channel──► control loop PipeWire (DaemonHost)
        ▲   метры 1–60 Гц                                │ MicControl::apply
   Hub (рассылка событий, насос метров)                  ▼ rtrb, без аллокаций
                                                     MicRt в RT-колбэке pw_filter
```

- `miceq-engine`: `MicControl` (control loop) и `MicRt` (RT). Непрерывные изменения идут
  в очередь `rtrb` и применяются `retune` со сглаживанием 10 мс. Структурные изменения
  (вкл/выкл модуля, bypass, look-ahead) собирают новый процессор вне RT; `MicRt` делает
  кроссфейд 10 мс, старый процессор возвращается второй очередью и освобождается на control
  loop (NFR-S2, DSP-003). Под `assert_no_alloc` проверены retune, замена и переполнение очереди.
- Задержка цепочки после изменений сообщается графу обновлением `ProcessLatency` (NFR-L4).
- Смена микрофона применяется без перезапуска: линк пересоздаётся на следующем тике (100 мс).
  Отключение/подключение входа рассылается событием `mic.changed` (основа тоста MIC-005).
- Метры считаются в RT, только пока есть подписчик (NFR-R7, MTR-004); иначе — ноль работы.
- Медленный клиент не блокирует демон: исходящие строки идут через ограниченные очереди,
  события для него отбрасываются.
- Потоки IPC стартуют после блокировки SIGINT/SIGTERM, поэтому сигналы обрабатывает
  только control loop и демон корректно завершается.

## Протокол (`miceq-ipc`, версия 1)

JSON по строкам, сокет `$XDG_RUNTIME_DIR/miceq/miceqd.sock` (каталог 0700, сокет 0600).
Первым запросом должен быть `hello` с номером протокола; при несовпадении — `protocol_mismatch`.

| Метод | Параметры | Ответ |
|---|---|---|
| `hello` | `protocol`, `client` | `protocol`, `daemon_version` |
| `mic.get` | — | `MicState` |
| `mic.set` | `settings` (вся цепочка) | `MicState` |
| `mic.set_preset` | `preset` (`natural`, `discord-clear`, …) | `MicState`; mute и bypass сохраняются |
| `mic.set_muted` / `mic.toggle_mute` | `muted` / — | `MicState` |
| `mic.set_bypass` | `bypass` | `MicState` |
| `mic.set_device` | `device` (`node.name` или `null`) | `MicState` |
| `presets.list`, `devices.list` | — | списки |
| `meters.subscribe` / `meters.unsubscribe` | `hz` (1–60) / — | `null`, затем события `meters` |

События: `meters` (уровни в dBFS, gain reduction в dB, активность NS) и `mic.changed` (любое
изменение состояния от любого клиента). Ошибки: `bad_request`, `protocol_mismatch`,
`invalid_params`, `not_found`, `busy`, `internal`.

Генерация TS-типов для UI из `miceq-ipc` ещё не сделана — появится вместе с UI.

## Конфиг (`miceq-config`)

`$XDG_CONFIG_HOME/miceq/config.toml`, `schema_version = 1`. Секция `[mic]`: `device`, `preset`,
`muted`, `bypass`, `output_gain_db` и таблицы модулей `[mic.compressor]` и т.д. Отсутствующие
поля берутся из значений по умолчанию. Пишется атомарно (temp + fsync + rename) через 500 мс
после последнего изменения и при выходе (CFG-001). Повреждённый, невалидный или более новый
по схеме файл переименовывается в `config.toml.broken-<unix time>`, демон стартует с
дефолтами (CFG-002; тоста пока нет — только предупреждение в логе).

Флаги `miceqd run` (`--input`, `--preset`, `--engine`, …) перекрывают конфиг на этот запуск;
последующие изменения через IPC сохраняются вместе с ними. `--config <path>` — другой файл.

Отличия от примера в PRD §13: модули лежат в `[mic.<module>]`, а не в `[mic.modules.<module>]`;
проценты — 0–100, а не доли; правила приложений, микшер и хоткеи появятся в M2.

## Не сделано

- Пользовательские пресеты, импорт/экспорт, калибровка, A/B — дальше по M1.
- systemd user unit и автозапуск, трей — дальше по M1 (§8, §15).
- Сохранение конфига, пока PipeWire недоступен, происходит только при выходе.

## Проверка

```sh
cargo test -p miceq-engine -p miceq-config -p miceq-ipc
cargo build --release -p miceqd -p miceq-cli
python3 tools/tests/control-smoke.py
```

`control-smoke.py` в приватном PipeWire проверяет: отказ второго демона, mute/unmute и gain
на лету, смену пресета с кроссфейдом и обновлением `ProcessLatency` в графе, метры, ошибки
протокола, сохранение конфига и его загрузку после перезапуска, удаление сокета при выходе.
