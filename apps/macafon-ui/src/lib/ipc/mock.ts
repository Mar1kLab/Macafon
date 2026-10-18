// In-browser stand-in for macafond (AGENTS.md §5): the same protocol with plausible devices,
// presets, live meters, calibration and A/B, so the UI runs without Tauri or PipeWire.
// `?daemon=down` simulates a daemon that cannot be started, `?mic=disconnected` an unplugged
// microphone, `?config=recovered` a config reset at startup, `?overload` a Studio overload,
// `?mixer=off` a mixer that is not turned on yet, `?hotkeys=unavailable` a desktop without a
// global shortcut service. `?platform=windows` acts like the Windows daemon (VB-CABLE, no
// channel effects); add `&cable=missing` for a system without VB-CABLE. `?look=sketch` picks the
// sketch look of the popup and overlay; `?sounds=list` and `?controls=bottom|hidden` lay out the
// overlay's soundpad.

import { DaemonError, type DaemonStatus, type Transport } from './client';
import { BUILTIN_PRESETS } from './dsp';
import type {
  AbState,
  AutostartState,
  CalibrationReport,
  CalibrationState,
  ChainSettings,
  DoctorCheck,
  DoctorReport,
  Event,
  HotkeyBinding,
  HotkeysState,
  InputChannel,
  Meters,
  MicState,
  Preferences,
  PresetInfo,
  OutputControl,
  MacaronSettings,
} from './protocol';
import { DEVICES, MOCK_ICONS } from './mock/fixtures';
import { clone, latencySamples, sameChain, slugify } from './mock/helpers';
import { MockMixer } from './mock/mixer';
import { MockSoundpad } from './mock/soundpad';
import { SILENT, random, voiceLevel } from './mock/signals';

const VERSION = '0.1.0-mock';

const GLOBAL_HOTKEYS = ['quick_panel', 'mic_mute_toggle', 'push_to_talk', 'push_to_mute', 'bypass_toggle', 'preset_next', 'preset_previous'];
const SOUNDPAD_HOTKEYS = ['soundpad_stop', 'soundpad_pause', 'soundpad_random'];

/** HK-004 command for an action, as `HotkeyAction::command` builds it. */
function hotkeyCommand(action: string): string {
  const [kind, channel] = action.split('.');
  switch (kind) {
    case 'quick_panel':
      return 'macafon quick-panel';
    case 'master_volume_up':
      return 'macafon ctl channel master volume +5';
    case 'master_volume_down':
      return 'macafon ctl channel master volume -5';
    case 'master_mute':
      return 'macafon ctl channel master mute';
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
    case 'soundpad_stop':
      return 'macafon ctl soundpad stop';
    case 'soundpad_pause':
      return 'macafon ctl soundpad pause';
    case 'soundpad_random':
      return 'macafon ctl soundpad random';
    default:
      if (action.startsWith('soundpad_play.')) return `macafon ctl soundpad play ${JSON.stringify(action.slice('soundpad_play.'.length))}`;
      return '';
  }
}

export class MockTransport implements Transport {
  private events = new Set<(event: Event) => void>();
  private statuses = new Set<(status: DaemonStatus) => void>();
  private status: DaemonStatus = { state: 'connecting' };
  private query = new URLSearchParams(location.search);
  private down = this.query.get('daemon') === 'down';
  private windows = this.query.get('platform') === 'windows';
  private cableMissing = this.windows && this.query.get('cable') === 'missing';

  private settings: ChainSettings = clone(BUILTIN_PRESETS[1]!.settings);
  private preset: string | null = BUILTIN_PRESETS[1]!.id;
  private device: string | null = DEVICES[0]!.name;
  private fallbackDevice: string | null = DEVICES[2]!.name;
  private connected = this.query.get('mic') !== 'disconnected';
  private inputChannel: InputChannel = 'Auto';
  private noiseFloor: number | null = null;
  private userPresets = new Map<string, { name: string; settings: ChainSettings }>();
  private isDefaultSource = false;
  private prefs: Preferences = {
    volume_step_pct: 5,
    quantum_frames: 128,
    close_action: 'tray',
    osd: { primary_monitor: false, style: 'vertical', edge: 'right', offset_pct: 0, look: this.query.get('look') === 'sketch' ? 'sketch' : 'classic' },
  };
  private autostart: AutostartState = { enabled: true, method: this.windows ? 'windows_run' : 'systemd' };
  private shortcuts = new Map<string, string>([
    // The daemon's defaults (DEFAULT_HOTKEYS in macafon-config) plus push to talk.
    ['quick_panel', 'LCtrl+LAlt+Z'],
    ['mic_mute_toggle', 'Ctrl+Alt+Q'],
    ['push_to_talk', 'F9'],
    ['channel_volume_up.main', 'RCtrl+PageUp'],
    ['channel_volume_down.main', 'RCtrl+PageDown'],
    ['channel_volume_up.voice', 'RAlt+PageUp'],
    ['channel_volume_down.voice', 'RAlt+PageDown'],
    ['channel_volume_up.music', 'Ctrl+Alt+R'],
    ['channel_volume_down.music', 'Ctrl+Alt+F'],
    ['soundpad_stop', 'Ctrl+Alt+X'],
    ['soundpad_play.Air horn.mp3', 'Ctrl+Alt+1'],
    ['soundpad_play.Memes/Bruh.mp3', 'Ctrl+Alt+2'],
    ['soundpad_play.Memes/Vine boom.wav', 'Ctrl+Alt+3'],
  ]);

  private meterTimer: number | undefined;
  private deviceMeterTimer: number | undefined;
  private started = performance.now();
  private mixer = new MockMixer(this.query.get('mixer') !== 'off', this.started, (event) => this.emit(event));
  private soundpad = new MockSoundpad((event) => this.emit(event), this.windows);
  private outputControls: OutputControl[] = [];
  private hardwareVolume = 83;
  private hardwareBalance = 0;

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
    if (this.query.has('macaron') && this.query.get('macaron') !== 'absent') {
      const device: OutputControl = {
        id: 'mock-macaron', name: 'iBasso Jr. Macaron', output: 'mock-macaron-output', error: null, live_volume: false,
        settings: { volume: null, balance: null, gain: null, filter: null, output_mode: null },
      };
      if (this.query.get('macaron') === 'known') {
        device.settings = { volume: 35, balance: 0, gain: 1, filter: 0, output_mode: 0 };
        this.hardwareVolume = 35;
      }
      if (this.query.get('macaron') !== 'unknown') {
        device.settings.volume = this.hardwareVolume; device.settings.balance = this.hardwareBalance; device.live_volume = true;
      }
      this.outputControls = [device];
      this.mixer.state.outputs.push({ name: device.output!, description: 'Headphones (Macaron)', kind: 'headphones' });
      this.mixer.state.output = device.output;
      if (this.query.get('macaron') === 'live') setTimeout(() => {
        this.hardwareVolume = 81; this.hardwareBalance = 0;
        device.settings.volume = 81; device.settings.balance = 0; device.live_volume = true;
        this.emit({ event: 'output_controls.changed', data: clone(this.outputControls) });
      }, 2000);
      if (this.query.get('macaron') === 'disconnect') setTimeout(() => {
        this.outputControls = [];
        this.emit({ event: 'output_controls.changed', data: [] });
      }, 8000);
    }
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
      is_default_source: this.isDefaultSource,
      virtual_mic_ready: !this.cableMissing,
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

  /** The system output sits on the cable until `doctor.fix` (WIN-MIC-004). */
  private cableDefaultOutput = true;

  private windowsDoctor(): DoctorReport {
    const cable: DoctorCheck[] = this.cableMissing
      ? [{ id: 'vb_cable', status: 'error', values: [] }]
      : [
          { id: 'vb_cable', status: 'ok', values: [] },
          { id: 'cable_format', status: 'ok', values: [] },
          { id: 'cable_writers', status: 'warning', values: ['obs64.exe'] },
          this.cableDefaultOutput
            ? { id: 'cable_default_output', status: 'warning', values: [], fixable: true }
            : { id: 'cable_default_output', status: 'ok', values: [] },
          { id: 'cable_latency', status: 'info', values: ['10.0', '2.7'] },
        ];
    return {
      daemon_version: VERSION,
      pipewire_version: null,
      session_manager: null,
      quantum: 480,
      sample_rate_hz: 48000,
      checks: [
        ...cable,
        this.connected
          ? { id: 'microphone_input', status: 'ok', values: [DEVICES[0]!.description] }
          : { id: 'microphone_input', status: 'error', values: [DEVICES[0]!.description] },
        this.mixer.state.enabled ? { id: 'mixer', status: 'ok', values: ['Speakers (Realtek(R) Audio)'] } : { id: 'mixer', status: 'info', values: [] },
        { id: 'processing', status: 'ok', values: ['480', '48000', '0'] },
        { id: 'hotkeys', status: 'ok', values: ['Windows'] },
      ],
    };
  }

  private hotkeys(): HotkeysState {
    const actions = [
      ...GLOBAL_HOTKEYS, 'master_volume_up', 'master_volume_down', 'master_mute',
      ...this.mixer.state.channels.flatMap((c) => [`channel_volume_up.${c.id}`, `channel_volume_down.${c.id}`, `channel_mute.${c.id}`]),
      ...(this.windows ? [...SOUNDPAD_HOTKEYS, ...this.soundpad.state.sounds.map((x) => `soundpad_play.${x.id}`)] : []),
    ];
    const portal = this.query.get('hotkeys') !== 'unavailable';
    return {
      backend: portal ? (this.windows ? 'windows' : 'portal') : 'unavailable',
      bindings: actions.map(
        (action): HotkeyBinding => ({
          action,
          secondary_shortcut: this.shortcuts.get(`${action}#secondary`) ?? null,
          secondary_error: null,
          shortcut: this.shortcuts.get(action) ?? null,
          // KDE shows its own spelling of what it registered.
          system_trigger: portal && this.shortcuts.has(action) ? (this.shortcuts.get(action) ?? null) : null,
          error: null,
          command: hotkeyCommand(action),
        }),
      ),
    };
  }

  /** Bindings of sounds that are gone are dropped; the mock does not track renames. */
  private syncSoundHotkeys() {
    const ids = new Set(this.soundpad.state.sounds.map((x) => `soundpad_play.${x.id}`));
    for (const action of [...this.shortcuts.keys()]) {
      if (action.startsWith('soundpad_play.') && !ids.has(action.replace(/#secondary$/, ''))) this.shortcuts.delete(action);
    }
    this.emit({ event: 'hotkeys.changed', data: this.hotkeys() });
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
    if (method.startsWith('mixer.') || method.startsWith('channel.') || (method.startsWith('apps.') && method !== 'apps.icon')) {
      return this.mixer.call(method, p);
    }
    if (method.startsWith('soundpad.')) {
      if (!this.windows) throw new DaemonError('bad_request', 'the soundpad is not available on this platform');
      const result = this.soundpad.call(method, p);
      // Renamed and moved sounds take their hotkeys along, as in the daemon.
      if (method === 'soundpad.rename' || method === 'soundpad.move' || method === 'soundpad.delete' || method.endsWith('_category')) {
        this.syncSoundHotkeys();
      }
      return result;
    }
    switch (method) {
      case 'output_controls.list': return clone(this.outputControls);
      case 'output_controls.set': {
        const device = this.outputControls.find((d) => d.id === p.id);
        if (!device) throw new DaemonError('not_found', 'Macaron disconnected');
        const patch = p.settings as MacaronSettings;
        const limits = { volume: [0, 100], balance: [-10, 10], gain: [0, 2], filter: [0, 4], output_mode: [0, 1] };
        for (const key of Object.keys(limits) as (keyof MacaronSettings)[]) {
          const v = patch[key];
          if (v != null && (!Number.isInteger(v) || v < limits[key][0]! || v > limits[key][1]!)) throw new DaemonError('invalid_params', 'Invalid Macaron settings');
        }
        if (this.query.get('macaron') === 'error') {
          device.error = 'Macaron acknowledgement timed out';
          device.settings = { volume: null, balance: null, gain: null, filter: null, output_mode: null };
          this.emit({ event: 'output_controls.changed', data: clone(this.outputControls) });
          throw new DaemonError('internal', device.error);
        }
        if (patch.volume != null || patch.balance != null) {
          this.hardwareVolume = patch.volume ?? this.hardwareVolume;
          this.hardwareBalance = patch.balance ?? this.hardwareBalance;
          device.settings.volume = this.hardwareVolume;
          device.settings.balance = this.hardwareBalance;
          device.live_volume = true;
        }
        for (const key of Object.keys(limits) as (keyof MacaronSettings)[]) {
          if (patch[key] != null) device.settings[key] = patch[key]!;
        }
        this.emit({ event: 'output_controls.changed', data: clone(this.outputControls) });
        return clone(this.outputControls);
      }
      case 'hello':
        return {
          protocol: 1,
          daemon_version: VERSION,
          platform: this.windows ? 'windows' : 'linux',
          capabilities: {
            virtual_mic_device: !this.windows,
            stereo_option: !this.windows,
            channel_fx: !this.windows,
            graph_quantum: !this.windows,
            hotkey_popup: this.windows,
            soundpad: this.windows,
          },
        };
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
        if (this.cableMissing) throw new DaemonError('internal', 'VB-CABLE is not installed');
        // PipeWire reports the new default a moment later, as a metadata change.
        setTimeout(() => {
          this.isDefaultSource = true;
          this.micChanged();
        }, 60);
        return null;
      case 'mic.restore_default_source':
        setTimeout(() => {
          this.isDefaultSource = false;
          this.micChanged();
        }, 60);
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
      case 'ui.window_closed':
        return null;
      case 'prefs.get':
        return clone(this.prefs);
      case 'prefs.set': {
        if (p.volume_step_pct != null) this.prefs.volume_step_pct = Math.max(1, Math.min(20, Number(p.volume_step_pct)));
        if (p.quantum_frames != null) this.prefs.quantum_frames = Number(p.quantum_frames);
        if (p.close_action != null) this.prefs.close_action = p.close_action as Preferences['close_action'];
        if (p.osd != null) this.prefs.osd = clone(p.osd as Preferences['osd']);
        this.emit({ event: 'prefs.changed', data: clone(this.prefs) });
        return clone(this.prefs);
      }
      case 'hotkeys.get':
        return this.hotkeys();
      case 'hotkeys.set': {
        const action = String(p.action) + (p.secondary ? '#secondary' : '');
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
      case 'apps.icon': {
        const svg = MOCK_ICONS[String(p.icon)];
        return svg ? `data:image/svg+xml;base64,${btoa(svg)}` : null;
      }
      case 'doctor.run':
        if (this.windows) return this.windowsDoctor();
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
            this.mixer.state.enabled
              ? { id: 'mixer', status: 'ok', values: ['Arctis Nova 7'] }
              : { id: 'mixer', status: 'info', values: [] },
            { id: 'unmovable_apps', status: 'ok', values: [] },
            { id: 'processing', status: 'ok', values: ['128', '48000', '0'] },
            { id: 'xruns', status: 'warning', values: ['2', '14'] },
            this.query.get('hotkeys') === 'unavailable'
              ? { id: 'hotkeys', status: 'info', values: [] }
              : { id: 'hotkeys', status: 'ok', values: ['portal'] },
          ],
        };
      case 'doctor.fix':
        this.cableDefaultOutput = false;
        return null;
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
      if (this.mixer.state.enabled) this.emit({ event: 'mixer.meters', data: this.mixer.meters() });
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
