# M2: микшер (каналы вывода)

Состояние на 2026-10-11. Готовы каналы, приложения и выходы (PRD §7) в демоне, CLI и UI; Doctor (§9.6);
горячие клавиши (§8.3) в демоне и CLI — раздел Hotkeys в UI следующий.

## Схема

[ADR-0003](../adr/0003-linux-channels-pw-filter-sinks.md): канал — узел `pw_filter`
`Audio/Sink` `macafon_ch_<id>` (описание «Macafon <Имя>»), стерео. Приложения подключаются к
входам, цепочка обрабатывает буфер в том же цикле, выходы демон линкует к реальному устройству.
Master — множитель громкости внутри каждого канала.

| Слой | Где |
|---|---|
| Цепочка `EQ → NS Lite (L/R) → Compressor → Limiter → Volume`, стерео-связка динамики, шаблоны CFX-005/006, пресеты EQ CFX-002, кубическая кривая CH-004 | `macafon-dsp/src/channel.rs` |
| Lock-free управление, кроссфейд при перестройке | `macafon-engine/src/channel.rs` |
| Ключ приложения (APP-002), имя, база известных приложений (APP-006), системные звуки (APP-011) | `macafon-apps` |
| Узлы каналов, линки к выходу, отслеживание потоков, `target.object` | `macafon-backend-pipewire/src/{channel_node,mixer}.rs` |
| Решения: какой канал, какой выход, default output | `macafond/src/mixer.rs` |
| Конфиг `[mixer]`, `[[mixer.channels]]`, `[[apps.rules]]` | `macafon-config/src/mixer.rs` |

## Поведение

- Микшер выключен по умолчанию (CH-009); включается в онбординге (шаг Mixer), на экране Mixer, в CLI.
- Включён: системный default output = канал по умолчанию (OUT-005). Прежний выход пишется в журнал
  `$XDG_STATE_HOME/macafon/default-sink.json` и возвращается при выключении, Quit, падении (при
  следующем старте) и `macafon restore-default`. После рестарта PipeWire журнал сохраняет исходный выход.
- Канал приложения: правило → встроенная классификация (если включена) → канал по умолчанию. Перенос
  чипа пишет правило и `target.object`; поток не перезапускается (APP-004/005). Удалённый канал → его
  правила переезжают в канал по умолчанию.
- Выход: первый присутствующий из приоритетного списка → прежний системный выход → текущий системный,
  если это реальное устройство → первое устройство. Подключение/отключение отрабатывается за ≤ 100 мс (OUT-002).
- Потоки с `node.dont-move` / `node.dont-reconnect` не трогаются; чип показывает замок.
- Свои потоки (`macafon_*`, A/B-плеер) не маршрутизируются; A/B при включённом микшере играет сразу
  в выходное устройство, а не в канал.

## CLI (HK-004)

```sh
macafon ctl mixer on|off|toggle          # без аргумента — состояние
macafon ctl channel voice volume +5      # -5, или абсолютное значение: 40
macafon ctl channel main mute [on|off|toggle]
macafon ctl channel music default
macafon ctl apps [list] | macafon ctl apps move bin:firefox music
macafon ctl output <node.name>|default
```

## Проверка

`python3 tools/tests/mixer-smoke.py` — приватные D-Bus/PipeWire/WirePlumber: узлы каналов и их линки,
default output и его возврат, классификация Discord → Voice, перенос без перезапуска и правило при
повторном запуске, звук на выходном устройстве, CLI, смена выхода, приоритет при подключении и
отключении устройства, выключение микшера, рестарт PipeWire, Quit без оставшихся узлов.

## Отклонения и пробелы

- Метр Master — максимум пиков каналов после Master, а не замер устройства.
- Правило применяется в течение ≤ 100 мс после появления потока: короткий звук в канале по умолчанию возможен.

## Трей и уведомления

Меню трея: переключатель Mixer и `Output device ▸` (BG-003). При первом закрытии окна демон показывает
уведомление freedesktop «Macafon keeps running in the tray» (BG-001, флаг `[ui] close_hint_shown`).
Проверка — `tools/tests/tray-smoke.py` (dbus-monitor в приватной сессии).

## Doctor (§9.6)

`doctor.run` / `macafon ctl doctor`: версии PipeWire и WirePlumber (снимок графа на отдельном соединении),
микрофон выбран/подключён, используется ли Macafon Microphone, сторонние шумодавы (EasyEffects,
NoiseTorch, RNNoise-фильтры), каналы слинкованы с выходом, неперемещаемые приложения, quantum и
переходы Studio → Lite за 10 минут. UI: Settings → Doctor с исправлениями в один клик (выбрать микрофон,
сделать default, включить микшер) и копированием отчёта без имён устройств и приложений.
Проверка `xruns`: Doctor на своём соединении подписывается на профайлер PipeWire на ≤ 300 мс и берёт
`SPA_PROFILER_info` драйвера, который обрабатывает Macafon Microphone: нагрузку DSP (среднее «medium») и
счётчик xrun. Отклонение от §9.6: счётчик — с момента запуска драйвера, а не за последние 10 минут;
без активной обработки микрофона или без `module-profiler` на сервере — «не измерено».

## Горячие клавиши (§8.3)

Регистрирует демон (HK-003): портал `org.freedesktop.portal.GlobalShortcuts` через zbus (новая сессия
на каждое изменение, desktop может назначить другие клавиши — `system_trigger`), иначе в X11-сессии —
`global-hotkey` (x11rb), иначе `unavailable` и CLI-команды (HK-004). Действия HK-001: mute, push-to-talk,
push-to-mute, bypass, следующий/предыдущий пресет, громкость ±5% и mute каждого канала. Конфиг
`[hotkeys]`: `mic_mute_toggle = "Ctrl+Alt+M"`, канальные — `channel_volume_up.voice`. Сочетание
нормализуется, дубликаты внутри Macafon отклоняются (HK-002). OSD-уведомление при смене пресета и
громкости (HK-006, одно уведомление заменяет предыдущее).

```sh
macafon ctl hotkeys [list] | macafon ctl hotkeys set push_to_talk F9 | … set <action> none
macafon ctl mic mute toggle | macafon ctl preset next|previous
```

Проверка — `tools/tests/hotkeys-smoke.py` с фейковым порталом `tools/tests/fake_portal.py` (python-dbus).
Не проверено: настоящие KDE/GNOME/Hyprland-порталы, X11-захват клавиш (нет XTEST на стенде; только
сборка).

Settings → Hotkeys (HK-002): действия по группам (микрофон, каждый канал), запись сочетания нажатием
(физические клавиши `KeyboardEvent.code`, та же каноническая форма, что у `Accel`), конфликт внутри
Macafon показывается до запроса к демону, `system_trigger` портала — подписью. Без портала и X11
вместо записи — CLI-команды с копированием (HK-004).

## Громкость приложения (CH-008)

`apps.set_volume { app_key, volume_pct }` запоминается в правиле (`volume_pct` в `[[apps.rules]]`, та же
кубическая кривая, что у фейдера канала). Демон пишет `SPA_PROP_volume` в Props узла каждого потока
приложения — это отдельный множитель, `channelVolumes` остаются за приложением и pipewire-pulse.
Приложения с «Don't manage», выключенный микшер и Quit возвращают множитель 1.0. UI: клик по фишке —
мини-слайдер, не 100% показывается на фишке.

## Иконки приложений (APP-003)

`apps.icon { icon }` → `data:` URL или `null`. Резолвер в `macafon-apps::icons`: `.desktop`-файлы
(по имени файла, бинарю из `Exec=`, `StartupWMClass`) → `Icon=` → тема hicolor, затем прочие темы
(`<size>/apps` и `apps/<size>`), затем `pixmaps`; SVG/PNG до 512 КиБ. Каталоги — `XDG_DATA_HOME`,
`XDG_DATA_DIRS`, экспорт Flatpak. Найденные иконки кешируются до перезапуска, промахи — на 60 с.
UI кеширует ответ по имени и до него рисует букву.
