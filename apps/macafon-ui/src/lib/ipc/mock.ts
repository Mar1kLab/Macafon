// In-browser stand-in for macafond (AGENTS.md §5): the same protocol with plausible devices,
// presets, live meters, calibration and A/B, so the UI runs without Tauri or PipeWire.
// `?daemon=down` simulates a daemon that cannot be started, `?mic=disconnected` an unplugged
// microphone, `?config=recovered` a config reset at startup, `?overload` a Studio overload,
// `?mixer=off` a mixer that is not turned on yet, `?hotkeys=unavailable` a desktop without a
// global shortcut service.

import { DaemonError, type DaemonStatus, type Transport } from './client';
import {
  BUILTIN_PRESETS,
  CHANNEL_TEMPLATES,
  LITE_LATENCY_SAMPLES,
  MAX_CHANNELS,
  SAMPLE_RATE_HZ,
  STUDIO_LATENCY_SAMPLES,
} from './dsp';
import type {
  AbState,
  AutostartState,
  CalibrationReport,
  CalibrationState,
  AppState,
  ChainSettings,
  ChannelSettings,
  ChannelState,
  Device,
  Event,
  HotkeyBinding,
  HotkeysState,
  InputChannel,
  Meters,
  MicState,
  MixerMeters,
  MixerState,
  PresetInfo,
} from './protocol';

const DEVICES: Device[] = [
  { name: 'alsa_input.usb-Blue_Microphones_Yeti_Stereo_Microphone-00.analog-stereo', description: 'Yeti Stereo Microphone', kind: 'usb' },
  { name: 'alsa_input.usb-SteelSeries_Arctis_Nova_7-00.mono-fallback', description: 'Arctis Nova 7 Headset', kind: 'headset' },
  { name: 'alsa_input.pci-0000_00_1f.3.analog-stereo', description: 'Built-in Audio Analog Stereo', kind: 'builtin' },
];

const SILENT = -120;

function channelLatency(settings: ChannelSettings): number {
  if (settings.bypass) return 0;
  return settings.noise_suppression.enabled ? LITE_LATENCY_SAMPLES : 0;
}

function channel(id: string, name: string, color: string, icon: string, settings: ChannelSettings): ChannelState {
  return { id, name, color, icon, settings: structuredClone(settings), latency_samples: channelLatency(settings) };
}

function app(key: string, name: string, channel: string, active: boolean, streams = 1): AppState {
  return { key, name, icon: null, channel, active, streams, managed: true, movable: true };
}

/** Plausible desktop: a browser and a game in Main, Discord in Voice, Cider in Music. */
function initialMixer(enabled: boolean): MixerState {
  return {
    enabled,
    default_channel: 'main',
    master_volume_pct: 100,
    master_muted: false,
    auto_classify: true,
    channels: [
      channel('main', 'Main', 'blue', 'speaker', CHANNEL_TEMPLATES.main),
      channel('voice', 'Voice', 'violet', 'message', CHANNEL_TEMPLATES.voice),
      channel('music', 'Music', 'pink', 'music', CHANNEL_TEMPLATES.music),
    ],
    apps: enabled
      ? [
          app('bin:firefox', 'Firefox', 'main', true, 2),
          app('name:Cyberpunk 2077', 'Cyberpunk 2077', 'main', true),
          app('system:sounds', 'System sounds', 'main', false),
          app('bin:discord', 'Discord', 'voice', true, 2),
          app('bin:telegram-desktop', 'Telegram', 'voice', false, 0),
          app('bin:cider', 'Cider', 'music', true),
        ]
      : [],
    outputs: [
      { name: 'alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo', description: 'Arctis Nova 7', kind: 'headphones' },
      { name: 'alsa_output.pci-0000_00_1f.3.analog-stereo', description: 'Built-in Speakers', kind: 'speakers' },
    ],
    output_priority: [],
    output: enabled ? 'alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo' : null,
  };
}
const VERSION = '0.1.0-mock';

const GLOBAL_HOTKEYS = ['mic_mute_toggle', 'push_to_talk', 'push_to_mute', 'bypass_toggle', 'preset_next', 'preset_previous'];

/** HK-004 command for an action, as `HotkeyAction::command` builds it. */
function hotkeyCommand(action: string): string {
  const [kind, channel] = action.split('.');
  switch (kind) {
    case 'mic_mute_toggle':
      return 'macafon ctl mic mute toggle';
    case 'bypass_toggle':
      return 'macafon ctl mic bypass toggle';
    case 'preset_next':
      return 'macafon ctl preset next';
    case 'preset_previous':
      return 'macafon ctl preset previous';
    case 'channel_volume_up':
      return `macafon ctl channel ${channel} volume +5`;
    case 'channel_volume_down':
      return `macafon ctl channel ${channel} volume -5`;
    case 'channel_mute':
      return `macafon ctl channel ${channel} mute`;
    default:
      return '';
  }
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

/** Chain equality without mute and bypass, which presets do not store. */
function sameChain(a: ChainSettings, b: ChainSettings): boolean {
  const strip = (s: ChainSettings) => JSON.stringify({ ...s, muted: false, bypass: false });
  return strip(a) === strip(b);
}

function latencySamples(settings: ChainSettings): number {
  if (settings.bypass) return 0;
  const engine = settings.noise_suppression.engine === 'studio' ? STUDIO_LATENCY_SAMPLES : LITE_LATENCY_SAMPLES;
  const ns = settings.noise_suppression.enabled ? engine : 0;
  const limiter = settings.limiter.enabled ? (settings.limiter.lookahead_ms * SAMPLE_RATE_HZ) / 1000 : 0;
  return ns + limiter;
}

function slugify(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'preset';
}

/** Deterministic pseudo-random sequence so screenshots are stable. */
function random(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 2 ** 32;
  };
}

/** Speech-like envelope in dBFS: syllables around −24 dBFS, pauses at the room floor. */
function voiceLevel(timeS: number, floorDbfs: number): number {
  const phrase = timeS % 6;
  if (phrase > 4.2) return floorDbfs + Math.sin(timeS * 31) * 1.5;
  const syllable = Math.max(0, Math.sin(timeS * Math.PI * 4.3)) ** 0.6;
  const accent = 0.6 + 0.4 * Math.sin(timeS * 1.7);
  const level = -42 + 20 * syllable * accent;
  return Math.max(level, floorDbfs);
}

export class MockTransport implements Transport {
  private events = new Set<(event: Event) => void>();
  private statuses = new Set<(status: DaemonStatus) => void>();
  private status: DaemonStatus = { state: 'connecting' };
  private query = new URLSearchParams(location.search);
  private down = this.query.get('daemon') === 'down';

  private settings: ChainSettings = clone(BUILTIN_PRESETS[1]!.settings);
  private preset: string | null = BUILTIN_PRESETS[1]!.id;
  private device: string | null = DEVICES[0]!.name;
  private fallbackDevice: string | null = DEVICES[2]!.name;
  private connected = this.query.get('mic') !== 'disconnected';
  private inputChannel: InputChannel = 'Auto';
  private noiseFloor: number | null = null;
  private userPresets = new Map<string, { name: string; settings: ChainSettings }>();
  private autostart: AutostartState = { enabled: true, method: 'systemd' };
  private shortcuts = new Map<string, string>([
    ['mic_mute_toggle', 'Ctrl+Alt+M'],
    ['push_to_talk', 'F9'],
    ['channel_volume_up.music', 'Ctrl+Alt+PageUp'],
    ['channel_volume_down.music', 'Ctrl+Alt+PageDown'],
  ]);

  private mixer: MixerState = initialMixer(this.query.get('mixer') !== 'off');
  private meterTimer: number | undefined;
  private deviceMeterTimer: number | undefined;
  private started = performance.now();

  private calibration: CalibrationState = { state: 'idle' };
  private calibrationTimer: number | undefined;

  private ab: AbState = {
    recording: null,
    clip_ms: null,
    playing: false,
    side: 'b',
    position_ms: 0,
    loudness_match: true,
    device: null,
  };
  private abTimer: number | undefined;

  constructor() {
    setTimeout(() => this.setStatus(this.down ? this.unavailable() : this.connectedStatus()), 250);
    if (this.query.has('overload')) {
      setTimeout(() => {
        this.settings.noise_suppression.engine = 'lite';
        this.emit({ event: 'mic.noise_fallback', data: { engine: 'lite' } });
        this.micChanged();
      }, 2000);
    }
  }

  private connectedStatus(): DaemonStatus {
    return { state: 'connected', version: VERSION };
  }

  private unavailable(): DaemonStatus {
    return { state: 'unavailable', reason: 'macafond exited: PipeWire is not running (mock)' };
  }

  private setStatus(status: DaemonStatus) {
    this.status = status;
    this.statuses.forEach((handler) => handler(status));
  }

  private emit(event: Event) {
    this.events.forEach((handler) => handler(clone(event)));
  }

  onEvent(handler: (event: Event) => void): () => void {
    this.events.add(handler);
    return () => this.events.delete(handler);
  }

  onStatus(handler: (status: DaemonStatus) => void): () => void {
    this.statuses.add(handler);
    handler(this.status);
    return () => this.statuses.delete(handler);
  }

  retry(): void {
    this.setStatus({ state: 'connecting' });
    setTimeout(() => this.setStatus(this.down ? this.unavailable() : this.connectedStatus()), 600);
  }

  // --- State ------------------------------------------------------------------------------

  private presets(): PresetInfo[] {
    return [
      ...BUILTIN_PRESETS.map((p) => ({ id: p.id, name: p.name, builtin: true })),
      ...[...this.userPresets].map(([id, p]) => ({ id, name: p.name, builtin: false })),
    ];
  }

  private presetSettings(id: string): ChainSettings | undefined {
    return BUILTIN_PRESETS.find((p) => p.id === id)?.settings ?? this.userPresets.get(id)?.settings;
  }

  private mic(): MicState {
    const presetSettings = this.preset ? this.presetSettings(this.preset) : undefined;
    return {
      stereo: false,
      input_channel: this.inputChannel,
      ...(this.query.get('config') === 'recovered'
        ? { config_recovery: { backup: '~/.config/macafon/config.toml.broken-1760180000', reason: 'TOML parse error at line 12' } }
        : {}),
      settings: clone(this.settings),
      preset: this.preset,
      preset_modified: presetSettings ? !sameChain(presetSettings, this.settings) : false,
      device: this.device,
      fallback_device: this.fallbackDevice,
      active_device: this.connected ? this.device : null,
      input_connected: this.connected && this.device !== null,
      latency_samples: latencySamples(this.settings),
      noise_floor_dbfs: this.noiseFloor,
    };
  }

  private micChanged(): MicState {
    const state = this.mic();
    this.emit({ event: 'mic.changed', data: state });
    return state;
  }

  private presetsChanged() {
    this.emit({ event: 'presets.changed', data: this.presets() });
  }

  private hotkeys(): HotkeysState {
    const actions = [
      ...GLOBAL_HOTKEYS,
      ...this.mixer.channels.flatMap((c) => [`channel_volume_up.${c.id}`, `channel_volume_down.${c.id}`, `channel_mute.${c.id}`]),
    ];
    const portal = this.query.get('hotkeys') !== 'unavailable';
    return {
      backend: portal ? 'portal' : 'unavailable',
      bindings: actions.map(
        (action): HotkeyBinding => ({
          action,
          shortcut: this.shortcuts.get(action) ?? null,
          // KDE shows its own spelling of what it registered.
          system_trigger: portal && this.shortcuts.has(action) ? (this.shortcuts.get(action) ?? null) : null,
          error: null,
          command: hotkeyCommand(action),
        }),
      ),
    };
  }

  private abChanged(): AbState {
    this.emit({ event: 'ab.changed', data: clone(this.ab) });
    return clone(this.ab);
  }

  private uniqueName(name: string): string {
    const taken = (candidate: string) =>
      this.presets().some((p) => p.name.toLowerCase() === candidate.toLowerCase());
    if (!taken(name)) return name;
    for (let n = 2; ; n++) {
      if (!taken(`${name} (${n})`)) return `${name} (${n})`;
    }
  }

  private addPreset(name: string, settings: ChainSettings): PresetInfo {
    let slug = slugify(name);
    while (this.userPresets.has(`user:${slug}`)) slug += '-2';
    const id = `user:${slug}`;
    this.userPresets.set(id, { name, settings: clone({ ...settings, muted: false, bypass: false }) });
    this.presetsChanged();
    return { id, name, builtin: false };
  }

  // --- Requests ---------------------------------------------------------------------------

  async call(method: string, params: unknown): Promise<unknown> {
    if (this.status.state !== 'connected') {
      throw new DaemonError('unavailable', 'macafond is not running');
    }
    // A real socket round trip is asynchronous too.
    await new Promise((resolve) => setTimeout(resolve, 4));
    const p = (params ?? {}) as Record<string, unknown>;
    switch (method) {
      case 'hello':
        return { protocol: 1, daemon_version: VERSION };
      case 'daemon.quit':
        this.setStatus(this.unavailable());
        return null;
      case 'autostart.get':
        return clone(this.autostart);
      case 'autostart.set':
        this.autostart.enabled = Boolean(p.enabled);
        return clone(this.autostart);
      case 'mic.get':
        return this.mic();
      case 'mic.set': {
        const settings = p.settings as ChainSettings;
        this.settings = clone(settings);
        return this.micChanged();
      }
      case 'mic.set_preset': {
        const settings = this.presetSettings(String(p.preset));
        if (!settings) throw new DaemonError('not_found', `no preset ${String(p.preset)}`);
        this.settings = { ...clone(settings), muted: this.settings.muted, bypass: this.settings.bypass };
        this.preset = String(p.preset);
        return this.micChanged();
      }
      case 'mic.set_muted':
        this.settings.muted = Boolean(p.muted);
        return this.micChanged();
      case 'mic.toggle_mute':
        this.settings.muted = !this.settings.muted;
        return this.micChanged();
      case 'mic.set_bypass':
        this.settings.bypass = Boolean(p.bypass);
        return this.micChanged();
      case 'mic.set_device':
        if (this.device !== p.device) this.noiseFloor = null;
        this.device = (p.device as string | null) ?? null;
        this.connected = true;
        return this.micChanged();
      case 'mic.set_fallback_device':
        this.fallbackDevice = (p.device as string | null) ?? null;
        return this.micChanged();
      case 'mic.set_input_channel':
        this.inputChannel = p.channel as InputChannel;
        return this.micChanged();
      case 'mic.set_default_source':
        if (p.confirmed !== true) throw new DaemonError('invalid_params', 'explicit confirmation is required');
        return null;
      case 'mic.restore_default_source':
        return null;
      case 'presets.list':
        return this.presets();
      case 'presets.save': {
        const name = String(p.name).trim();
        if (this.presets().some((preset) => preset.name.toLowerCase() === name.toLowerCase())) {
          throw new DaemonError('already_exists', `a preset named ${name} already exists`);
        }
        const info = this.addPreset(name, this.settings);
        this.preset = info.id;
        this.micChanged();
        return info;
      }
      case 'presets.update': {
        const id = String(p.id);
        const preset = this.userPresets.get(id);
        if (!preset) throw new DaemonError('invalid_params', 'built-in presets are read-only');
        preset.settings = clone({ ...this.settings, muted: false, bypass: false });
        this.preset = id;
        this.micChanged();
        return { id, name: preset.name, builtin: false };
      }
      case 'presets.rename': {
        const preset = this.userPresets.get(String(p.id));
        if (!preset) throw new DaemonError('invalid_params', 'built-in presets are read-only');
        preset.name = String(p.name);
        this.presetsChanged();
        return { id: String(p.id), name: preset.name, builtin: false };
      }
      case 'presets.duplicate': {
        const id = String(p.id);
        const source = this.presets().find((preset) => preset.id === id);
        const settings = this.presetSettings(id);
        if (!source || !settings) throw new DaemonError('not_found', `no preset ${id}`);
        return this.addPreset(this.uniqueName(source.name), settings);
      }
      case 'presets.delete': {
        const id = String(p.id);
        if (!this.userPresets.delete(id)) throw new DaemonError('invalid_params', 'built-in presets are read-only');
        if (this.preset === id) this.preset = null;
        this.presetsChanged();
        this.micChanged();
        return null;
      }
      case 'presets.export': {
        const id = String(p.id);
        const source = this.presets().find((preset) => preset.id === id);
        const settings = this.presetSettings(id);
        if (!source || !settings) throw new DaemonError('not_found', `no preset ${id}`);
        return { schema_version: 1, name: source.name, settings: clone(settings) };
      }
      case 'presets.import': {
        const document = p.document as { name?: string; settings?: ChainSettings };
        if (!document?.name || !document.settings) throw new DaemonError('invalid_params', 'not a Macafon preset');
        return this.addPreset(this.uniqueName(document.name), document.settings);
      }
      case 'mixer.get':
        return clone(this.mixer);
      case 'mixer.set_enabled':
        this.mixer.enabled = Boolean(p.enabled);
        if (this.mixer.enabled && this.mixer.apps.length === 0) this.mixer.apps = initialMixer(true).apps;
        this.mixer.output = this.mixer.enabled ? (this.mixer.output_priority[0] ?? this.mixer.outputs[0]!.name) : null;
        return this.mixerChanged();
      case 'mixer.set_master':
        if (typeof p.volume_pct === 'number') this.mixer.master_volume_pct = p.volume_pct;
        if (typeof p.muted === 'boolean') this.mixer.master_muted = p.muted;
        return this.mixerChanged();
      case 'mixer.set_output': {
        const device = (p.device as string | null) ?? null;
        this.mixer.output_priority = device ? [device, ...this.mixer.output_priority.filter((d) => d !== device)] : [];
        this.mixer.output = this.mixer.enabled ? (device ?? this.mixer.outputs[0]!.name) : null;
        return this.mixerChanged();
      }
      case 'mixer.set_output_priority':
        this.mixer.output_priority = p.devices as string[];
        return this.mixerChanged();
      case 'mixer.set_auto_classify':
        this.mixer.auto_classify = Boolean(p.enabled);
        return this.mixerChanged();
      case 'channel.create': {
        if (this.mixer.channels.length >= MAX_CHANNELS) throw new DaemonError('invalid_params', 'too many channels');
        let n = 1;
        while (this.mixer.channels.some((c) => c.id === `ch-${n}`)) n++;
        this.mixer.channels.push(
          channel(`ch-${n}`, String(p.name), String(p.color), (p.icon as string | null) ?? 'speaker', CHANNEL_TEMPLATES.custom),
        );
        return this.mixerChanged();
      }
      case 'channel.update': {
        const target = this.channel(String(p.id));
        Object.assign(target, p.patch as object);
        return this.mixerChanged();
      }
      case 'channel.delete': {
        const id = String(p.id);
        if (id === this.mixer.default_channel) throw new DaemonError('invalid_params', 'the default channel cannot be deleted');
        this.mixer.channels = this.mixer.channels.filter((c) => c.id !== id);
        this.mixer.apps.forEach((a) => a.channel === id && (a.channel = this.mixer.default_channel));
        return this.mixerChanged();
      }
      case 'channel.set': {
        const target = this.channel(String(p.id));
        target.settings = clone(p.settings as ChannelSettings);
        target.latency_samples = channelLatency(target.settings);
        return this.mixerChanged();
      }
      case 'channel.adjust_volume': {
        const target = this.channel(String(p.id));
        target.settings.volume_pct = Math.min(Math.max(target.settings.volume_pct + Number(p.delta_pct), 0), 100);
        return this.mixerChanged();
      }
      case 'channel.toggle_mute': {
        const target = this.channel(String(p.id));
        target.settings.muted = !target.settings.muted;
        return this.mixerChanged();
      }
      case 'channel.set_default':
        this.channel(String(p.id));
        this.mixer.default_channel = String(p.id);
        return this.mixerChanged();
      case 'channel.reorder': {
        const ids = p.ids as string[];
        this.mixer.channels.sort((a, b) => ids.indexOf(a.id) - ids.indexOf(b.id));
        return this.mixerChanged();
      }
      case 'apps.move': {
        this.channel(String(p.channel));
        const target = this.mixer.apps.find((a) => a.key === p.app_key);
        if (target) {
          target.channel = String(p.channel);
          target.managed = true;
        }
        return this.mixerChanged();
      }
      case 'apps.set_managed': {
        const target = this.mixer.apps.find((a) => a.key === p.app_key);
        if (target) target.managed = Boolean(p.managed);
        return this.mixerChanged();
      }
      case 'apps.forget':
        this.mixer.apps = this.mixer.apps.filter((a) => a.key !== p.app_key || a.streams > 0);
        return this.mixerChanged();
      case 'hotkeys.get':
        return this.hotkeys();
      case 'hotkeys.set': {
        const action = String(p.action);
        const shortcut = (p.shortcut as string | null) ?? null;
        if (shortcut === null) {
          this.shortcuts.delete(action);
        } else {
          const other = [...this.shortcuts].find(([a, s]) => a !== action && s === shortcut);
          if (other) throw new DaemonError('already_exists', `${shortcut} is already used by ${other[0]}`);
          this.shortcuts.set(action, shortcut);
        }
        const state = this.hotkeys();
        this.emit({ event: 'hotkeys.changed', data: clone(state) });
        return state;
      }
      case 'doctor.run':
        return {
          daemon_version: VERSION,
          pipewire_version: '1.6.2',
          session_manager: 'WirePlumber 0.5.13',
          quantum: 128,
          sample_rate_hz: 48000,
          checks: [
            { id: 'pipewire', status: 'ok', values: ['1.6.2'] },
            { id: 'session_manager', status: 'ok', values: ['0.5.13'] },
            this.connected
              ? { id: 'microphone_input', status: 'ok', values: [DEVICES[0]!.description] }
              : { id: 'microphone_input', status: 'error', values: [DEVICES[0]!.description] },
            { id: 'microphone_used', status: 'info', values: [] },
            { id: 'double_processing', status: 'warning', values: ['Easy Effects Source'] },
            this.mixer.enabled
              ? { id: 'mixer', status: 'ok', values: ['Arctis Nova 7'] }
              : { id: 'mixer', status: 'info', values: [] },
            { id: 'unmovable_apps', status: 'ok', values: [] },
            { id: 'processing', status: 'ok', values: ['128', '48000', '0'] },
            this.query.get('hotkeys') === 'unavailable'
              ? { id: 'hotkeys', status: 'info', values: [] }
              : { id: 'hotkeys', status: 'ok', values: ['portal'] },
          ],
        };
      case 'devices.list':
        return clone(DEVICES);
      case 'devices.subscribe':
        clearInterval(this.deviceMeterTimer);
        this.deviceMeterTimer = window.setInterval(() => this.emitDeviceMeters(), 1000 / 15);
        return null;
      case 'devices.unsubscribe':
        clearInterval(this.deviceMeterTimer);
        this.deviceMeterTimer = undefined;
        return null;
      case 'meters.subscribe':
        this.subscribeMeters(Number(p.hz));
        return null;
      case 'meters.unsubscribe':
        clearInterval(this.meterTimer);
        this.meterTimer = undefined;
        return null;
      case 'calibration.start':
        if (this.ab.recording) throw new DaemonError('busy', 'a recording is in progress');
        this.startCalibration();
        return clone(this.calibration);
      case 'calibration.cancel':
        clearInterval(this.calibrationTimer);
        this.calibration = { state: 'idle' };
        this.emit({ event: 'calibration.changed', data: clone(this.calibration) });
        return clone(this.calibration);
      case 'calibration.get':
        return clone(this.calibration);
      case 'ab.record':
        if (this.calibration.state === 'measuring') throw new DaemonError('busy', 'calibration is running');
        this.startRecording(Number(p.max_seconds ?? 10));
        return this.abChanged();
      case 'ab.stop_recording':
        if (this.ab.recording && this.ab.recording.elapsed_ms >= 3000) this.finishRecording();
        return this.abChanged();
      case 'ab.play':
        if (this.ab.clip_ms === null) throw new DaemonError('invalid_params', 'nothing recorded');
        this.ab.playing = true;
        this.ab.device = (p.device as string | null) ?? null;
        this.runAbTimer();
        return this.abChanged();
      case 'ab.stop':
        this.ab.playing = false;
        return this.abChanged();
      case 'ab.select':
        this.ab.side = p.side === 'a' ? 'a' : 'b';
        return this.abChanged();
      case 'ab.toggle':
        this.ab.side = this.ab.side === 'a' ? 'b' : 'a';
        return this.abChanged();
      case 'ab.seek':
        this.ab.position_ms = Math.min(Number(p.position_ms), this.ab.clip_ms ?? 0);
        return this.abChanged();
      case 'ab.set_loudness_match':
        this.ab.loudness_match = Boolean(p.enabled);
        return this.abChanged();
      case 'ab.get':
        return clone(this.ab);
      case 'ab.waveform':
        return this.waveform(Number(p.points));
      default:
        throw new DaemonError('bad_request', `unknown method ${method}`);
    }
  }

  private channel(id: string): ChannelState {
    const found = this.mixer.channels.find((c) => c.id === id);
    if (!found) throw new DaemonError('not_found', `no channel ${id}`);
    return found;
  }

  private mixerChanged(): MixerState {
    this.emit({ event: 'mixer.changed', data: clone(this.mixer) });
    return clone(this.mixer);
  }

  // --- Meters -----------------------------------------------------------------------------

  /** Channel peaks after their faders; voices come and go, music keeps going. */
  private mixerMeters(): MixerMeters {
    const t = (performance.now() - this.started) / 1000;
    const master = this.mixer.master_muted ? 0 : (this.mixer.master_volume_pct / 100) ** 3;
    const source: Record<string, number> = {
      main: -14 + 4 * Math.sin(t * 2.1),
      voice: voiceLevel(t + 1.7, -70) + 14,
      music: -9 + 3 * Math.abs(Math.sin(t * 7.3)),
    };
    const channels = this.mixer.channels.map((c) => {
      const playing = this.mixer.apps.some((a) => a.channel === c.id && a.active);
      const gain = c.settings.muted ? 0 : (c.settings.volume_pct / 100) ** 3 * master;
      const level = playing && gain > 0 ? (source[c.id] ?? -18) + 20 * Math.log10(gain) : SILENT;
      return {
        id: c.id,
        meters: {
          peak_left_dbfs: level,
          peak_right_dbfs: level - (c.id === 'music' ? 1.5 * Math.sin(t * 3) : 0.5),
          compressor_reduction_db: 0,
          limiter_reduction_db: 0,
        },
      };
    });
    const max = (side: 'peak_left_dbfs' | 'peak_right_dbfs') =>
      channels.reduce((peak, c) => Math.max(peak, c.meters[side]), SILENT);
    return { channels, master_left_dbfs: max('peak_left_dbfs'), master_right_dbfs: max('peak_right_dbfs') };
  }

  /** The headset picks up the voice from across the desk; the built-in mic hears the room. */
  private emitDeviceMeters() {
    const t = (performance.now() - this.started) / 1000;
    const offsets = [0, -10, -30];
    DEVICES.forEach((device, i) => {
      const unplugged = !this.connected && device.name === this.device;
      const peak_dbfs = unplugged ? SILENT : voiceLevel(t, -66 + i * 4) + 6 + offsets[i]!;
      this.emit({ event: 'devices.meter', data: { name: device.name, peak_dbfs } });
    });
  }

  private subscribeMeters(hz: number) {
    clearInterval(this.meterTimer);
    this.meterTimer = window.setInterval(() => {
      this.emit({ event: 'meters', data: this.meters() });
      if (this.mixer.enabled) this.emit({ event: 'mixer.meters', data: this.mixerMeters() });
    }, 1000 / hz);
  }

  private meters(): Meters {
    const t = (performance.now() - this.started) / 1000;
    const s = this.settings;
    const floor = -63;
    let raw = this.connected ? voiceLevel(t, floor) : SILENT;
    if (this.calibration.state === 'measuring' && this.calibration.step === 'silence') raw = floor;
    const rawPeak = raw + 6;
    const gain = s.bypass || !s.gain.enabled ? 0 : s.gain.gain_db;
    const input = raw + gain;
    const zero: Meters = {
      raw_peak_dbfs: rawPeak,
      input_peak_dbfs: input + 6,
      input_rms_dbfs: input,
      output_peak_dbfs: SILENT,
      output_rms_dbfs: SILENT,
      expander_reduction_db: 0,
      compressor_reduction_db: 0,
      de_esser_reduction_db: 0,
      limiter_reduction_db: 0,
      noise_activity: 0,
    };
    if (raw <= SILENT) return zero;
    if (s.bypass) {
      return { ...zero, output_peak_dbfs: s.muted ? SILENT : input + 6, output_rms_dbfs: s.muted ? SILENT : input };
    }
    let level = input;
    const speaking = raw > floor + 10;
    let activity = 0;
    if (s.noise_suppression.enabled) {
      activity = speaking ? 0.15 : 0.6 + 0.3 * (s.noise_suppression.intensity_pct / 100);
      if (!speaking) level -= 25 * (s.noise_suppression.intensity_pct / 100);
    }
    let expander = 0;
    if (s.expander.enabled && level < s.expander.threshold_db) {
      expander = Math.max(s.expander.range_db, (level - s.expander.threshold_db) * 1.2);
      level += expander;
    }
    let compressor = 0;
    if (s.compressor.enabled && level > s.compressor.threshold_db) {
      const over = level - s.compressor.threshold_db;
      compressor = -(over - over / s.compressor.ratio);
      level += compressor + (s.compressor.makeup_db ?? 6);
    }
    const deEsser = s.de_esser.enabled && speaking ? -Math.max(0, Math.sin(t * 9)) * 3 : 0;
    level += deEsser;
    level += s.output_gain_db;
    let peak = level + 6;
    let limiter = 0;
    if (s.limiter.enabled && peak > s.limiter.ceiling_db) {
      limiter = s.limiter.ceiling_db - peak;
      peak = s.limiter.ceiling_db;
    }
    const muted = s.muted;
    return {
      ...zero,
      output_peak_dbfs: muted ? SILENT : peak,
      output_rms_dbfs: muted ? SILENT : level,
      expander_reduction_db: expander,
      compressor_reduction_db: compressor,
      de_esser_reduction_db: deEsser,
      limiter_reduction_db: limiter,
      noise_activity: activity,
    };
  }

  // --- Calibration --------------------------------------------------------------------------

  private startCalibration() {
    clearInterval(this.calibrationTimer);
    const begun = performance.now();
    const step = (elapsed: number): CalibrationState => {
      if (elapsed < 4000) {
        return { state: 'measuring', step: 'silence', progress: elapsed / 4000, step_duration_ms: 4000 };
      }
      return { state: 'measuring', step: 'speech', progress: Math.min((elapsed - 4000) / 6000, 1), step_duration_ms: 6000 };
    };
    this.calibration = step(0);
    this.calibrationTimer = window.setInterval(() => {
      const elapsed = performance.now() - begun;
      if (elapsed >= 10_000) {
        clearInterval(this.calibrationTimer);
        this.finishCalibration();
        return;
      }
      this.calibration = step(elapsed);
      this.emit({ event: 'calibration.changed', data: clone(this.calibration) });
    }, 100);
  }

  private finishCalibration() {
    const report: CalibrationReport = {
      noise_floor_dbfs: -63.2,
      speech_rms_dbfs: -27.5,
      speech_peak_dbfs: -11.8,
      snr_db: 35.7,
      snr_after_db: 61.4,
      noise_level: 'medium',
      low_frequency_noise: true,
      suggest_studio: false,
      gain_db: 9.5,
      high_pass_freq_hz: 120,
      noise_intensity_pct: 51,
      expander_threshold_db: -47.7,
    };
    const s = this.settings;
    s.gain = { ...s.gain, enabled: true, gain_db: report.gain_db };
    s.high_pass = { ...s.high_pass, enabled: true, freq_hz: report.high_pass_freq_hz };
    s.noise_suppression = { ...s.noise_suppression, enabled: true, intensity_pct: report.noise_intensity_pct };
    s.expander = { ...s.expander, enabled: true, threshold_db: report.expander_threshold_db };
    this.noiseFloor = report.noise_floor_dbfs;
    this.calibration = { state: 'done', report };
    this.emit({ event: 'calibration.changed', data: clone(this.calibration) });
    this.micChanged();
  }

  // --- A/B ----------------------------------------------------------------------------------

  private startRecording(maxSeconds: number) {
    this.ab.playing = false;
    this.ab.recording = { elapsed_ms: 0, max_ms: Math.min(Math.max(maxSeconds, 3), 30) * 1000 };
    this.runAbTimer();
  }

  private finishRecording() {
    if (!this.ab.recording) return;
    this.ab.clip_ms = this.ab.recording.elapsed_ms;
    this.ab.recording = null;
    this.ab.position_ms = 0;
  }

  private runAbTimer() {
    if (this.abTimer !== undefined) return;
    let last = performance.now();
    this.abTimer = window.setInterval(() => {
      const now = performance.now();
      const dt = now - last;
      last = now;
      if (this.ab.recording) {
        this.ab.recording.elapsed_ms = Math.min(this.ab.recording.elapsed_ms + dt, this.ab.recording.max_ms);
        if (this.ab.recording.elapsed_ms >= this.ab.recording.max_ms) this.finishRecording();
      } else if (this.ab.playing && this.ab.clip_ms) {
        this.ab.position_ms = (this.ab.position_ms + dt) % this.ab.clip_ms;
      } else {
        clearInterval(this.abTimer);
        this.abTimer = undefined;
      }
      this.ab.position_ms = Math.round(this.ab.position_ms);
      if (this.ab.recording) this.ab.recording.elapsed_ms = Math.round(this.ab.recording.elapsed_ms);
      this.abChanged();
    }, 100);
  }

  private waveform(points: number) {
    const next = random(7);
    const seconds = (this.ab.clip_ms ?? 10_000) / 1000;
    const a: number[] = [];
    const b: number[] = [];
    for (let i = 0; i < points; i++) {
      const t = (i / points) * seconds;
      const db = voiceLevel(t, -60) + (next() - 0.5) * 6;
      // Peaks are about 18 dB above the level; B is compressed, gated and limited.
      const amp = 10 ** ((db + 18) / 20);
      a.push(Math.min(amp + 0.02 + next() * 0.015, 1));
      b.push(db > -50 ? Math.min(Math.sqrt(amp) * 0.7, 0.89) : 0.003);
    }
    return { a, b };
  }
}
