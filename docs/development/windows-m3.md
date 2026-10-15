# Windows M3.0: spike

Состояние на 2026-10-15. Начат M3.0 из PRD §19.1. Демон на Windows пока не собирается;
готовы платформенные части в `crates/macafon-backend-windows` и утилита
`examples/spike.rs`. Стенд: Windows 11 25H2, HyperX SoloCast (USB), VB-CABLE 3.3.1.7.

```sh
cargo run --release -p macafon-backend-windows --example spike -- devices    # эндпоинты, hardware ID, периоды
cargo run --release -p macafon-backend-windows --example spike -- defaults   # default-устройства по ролям
cargo run --release -p macafon-backend-windows --example spike -- sessions   # аудиосессии приложений
cargo run --release -p macafon-backend-windows --example spike -- run --input SoloCast --seconds 60
```

`run` по умолчанию пишет в `CABLE Input`; `--output '<часть имени>'` выбирает другое устройство.
`set-default` меняет default-устройство и через несколько секунд восстанавливает прежнее.

## Что сделано

- Обнаружение VB-CABLE по hardware ID адаптера `VBAudioVACWDM` (служба `VBAudioVACMME`),
  без имён эндпоинтов (WIN-MIC-001). Адаптер — родитель devnode `SWD\MMDEVAPI\<id>`.
  VB-CABLE 3.3 даёт два render-эндпоинта на одном кабеле: `CABLE Input` (form factor
  Speakers) и `CABLE In 16ch` (LineLevel); выбирается Speakers. Прочие VB-Audio-устройства
  помечаются, но не используются.
- Default-устройства: чтение через `IMMDeviceEnumerator`, запись через `IPolicyConfig`
  (Windows 7+ vtable) — основа MIC-012/WIN-CH-007. Журнал и восстановление — M3.1.
- Аудиосессии на всех render-эндпоинтах: процесс, exe, громкость, mute, пик (основа
  WIN-CH-001/002/004), только чтение.
- Путь микрофона: WASAPI shared event-driven захват (MMCSS «Pro Audio») → `MicRt` →
  буфер компенсации дрейфа → WASAPI render, моно дублируется в стерео. Формат 48 кГц float32;
  ресемплинг и сведение каналов устройства делает Windows (`AUTOCONVERTPCM`). RT-участки
  под `assert_no_alloc`.
- `macafon-engine::drift`: SPSC-буфер + кубический ресемплер с PI-регулятором (≈ 20 с,
  до ±0,5 %). Регулируется суммарная задержка «буфер + очередь render»: render иногда
  забирает два периода сразу, и регулирование только по буферу давало серии underrun.

## Замеры

| | Результат |
|---|---|
| Период движка WASAPI по умолчанию | 10 мс у всех устройств; минимальный (IAudioClient3): `CABLE Input` 2,67 мс, HyperX 10 мс, `CABLE Output` 10 мс |
| Безопасный уровень буфера | буфер render (22 мс) + период (10 мс); меньше — underrun в части запусков |
| Задержка HyperX → `CABLE Input`, raw | ≈ 44 мс по оценке компонентов, без буфера VB-CABLE; не замерена петлёй |
| Дрейф HyperX ↔ VB-CABLE / Steam Speakers | десятки–сотни ppm после установления, точнее после soak-теста |
| Underrun / выпадения | 0 за 20–60 с при уровне по умолчанию |

**NFR-L6 (≤ 20 мс) с периодами 10 мс недостижим.** Следующий шаг — `IAudioClient3`
(`InitializeSharedAudioStream` с минимальным периодом) хотя бы для render в кабель; захват с
USB-микрофона остаётся 10 мс. Нужен замер петлёй (latency-probe для Windows) и часовой soak.

## Не сделано в M3.0

Замер задержки петлёй и буфера `VBCABLE_ControlPanel`, `IAudioClient3`, named pipe IPC,
события сессий (`IAudioSessionNotification`), hot-plug, проверка `IPolicyConfig` на Windows 10.
Цифры пока не внесены в ADR-0004.
