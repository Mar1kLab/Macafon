// UI state mirrored from the daemon. The UI only talks IPC (AGENTS.md §5): every change is a
// request, and the daemon's reply or event is the truth.

import { SvelteMap } from 'svelte/reactivity';
import { DaemonClient, DaemonError, type DaemonStatus, type Transport } from '../ipc/client';
import type {
  AbState,
  AutostartState,
  CalibrationState,
  ChainSettings,
  ChannelSettings,
  Device,
  Event,
  HotkeysState,
  MicState,
  MixerState,
  Preferences,
  PresetInfo,
} from '../ipc/protocol';
import { t } from '../i18n';
import { deviceMeterBus, meterBus, mixerMeterBus } from './meters';

export interface Toast {
  id: number;
  title: string;
  message?: string;
  tone: 'info' | 'danger';
}

/** Changes closer together than this form one undo step (a slider drag, a wheel spin). */
const UNDO_GESTURE_MS = 600;
const UNDO_DEPTH = 100;
const METER_HZ = 60;
const ONBOARDING_KEY = 'macafon.onboarding';

export class DaemonStore {
  readonly client: DaemonClient;
  status = $state<DaemonStatus>({ state: 'connecting' });
  mic = $state.raw<MicState | null>(null);
  presets = $state.raw<PresetInfo[]>([]);
  devices = $state.raw<Device[]>([]);
  ab = $state.raw<AbState | null>(null);
  calibration = $state.raw<CalibrationState>({ state: 'idle' });
  autostart = $state.raw<AutostartState | null>(null);
  mixer = $state.raw<MixerState | null>(null);
  hotkeys = $state.raw<HotkeysState | null>(null);
  prefs = $state.raw<Preferences | null>(null);
  toasts = $state<Toast[]>([]);
  canUndo = $state(false);
  /** Bumped to ask the TEST panel to record a clip (calibration → "Listen A/B"). */
  recordRequest = $state(0);
  canRedo = $state(false);
  /** First-run flow (PRD §9.2); finishing or skipping it is a UI preference of this WebView. */
  onboarding = $state(localStorage.getItem(ONBOARDING_KEY) !== 'done');

  private nextToast = 1;
  /** Settings waiting to be sent while a `mic.set` is in flight; the latest wins. */
  private queued: ChainSettings | null = null;
  private inFlight = false;
  private undoStack: ChainSettings[] = [];
  private redoStack: ChainSettings[] = [];
  private lastEditAt = 0;
  private metersOn = false;
  /** Components showing per-device levels; the daemon meters every input only while > 0. */
  private deviceWatchers = 0;
  private recoveryShown: string | null = null;
  /** APP-003: icon name → `data:` URL, `null` when the daemon found none. */
  private icons = new SvelteMap<string, string | null>();
  private iconsPending = new Set<string>();
  /** Channel settings waiting while a `channel.set` for that channel is in flight. */
  private channelQueue = new Map<string, ChannelSettings>();
  private channelInFlight = new Set<string>();

  constructor(
    transport: Transport,
    readonly mock: boolean,
  ) {
    this.client = new DaemonClient(transport);
    transport.onStatus((status) => this.onStatus(status));
    transport.onEvent((event) => this.onEvent(event));
    document.addEventListener('visibilitychange', () => this.syncMeters());
  }

  get connected(): boolean {
    return this.status.state === 'connected';
  }

  retry(): void {
    this.client.transport.retry();
  }

  // --- Connection -------------------------------------------------------------------------

  private onStatus(status: DaemonStatus) {
    const was = this.status.state;
    this.status = status;
    if (status.state === 'connected' && was !== 'connected') {
      this.metersOn = false;
      void this.load();
    }
    if (status.state !== 'connected') {
      this.metersOn = false;
      meterBus.reset();
      deviceMeterBus.reset();
      mixerMeterBus.reset();
    }
  }

  private async load() {
    try {
      const [mic, presets, devices, ab, calibration, autostart, mixer, hotkeys, prefs] = await Promise.all([
        this.client.call('mic.get'),
        this.client.call('presets.list'),
        this.client.call('devices.list'),
        this.client.call('ab.get'),
        this.client.call('calibration.get'),
        this.client.call('autostart.get'),
        this.client.call('mixer.get'),
        this.client.call('hotkeys.get'),
        this.client.call('prefs.get'),
      ]);
      this.prefs = prefs;
      this.mixer = mixer;
      this.hotkeys = hotkeys;
      this.applyMic(mic);
      this.presets = presets;
      this.devices = devices;
      this.ab = ab;
      this.calibration = calibration;
      this.autostart = autostart;
      this.syncMeters();
      if (this.deviceWatchers > 0) await this.client.call('devices.subscribe');
    } catch (error) {
      this.fail(error);
    }
  }

  /** Live levels of every microphone (MIC-002) until the returned function is called. */
  watchDevices(): () => void {
    if (this.deviceWatchers++ === 0 && this.connected) {
      this.client.call('devices.subscribe').catch((error) => this.fail(error));
    }
    let stopped = false;
    return () => {
      if (stopped) return;
      stopped = true;
      if (--this.deviceWatchers === 0) {
        deviceMeterBus.reset();
        if (this.connected) this.client.call('devices.unsubscribe').catch(() => {});
      }
    };
  }

  /** Meters flow only while the window is visible (MTR-004, NFR-R7). */
  private syncMeters() {
    const want = this.connected && document.visibilityState === 'visible';
    if (want === this.metersOn) return;
    this.metersOn = want;
    const request = want
      ? this.client.call('meters.subscribe', { hz: METER_HZ })
      : this.client.call('meters.unsubscribe');
    request.catch((error) => {
      this.metersOn = !want;
      this.fail(error);
    });
    if (!want) meterBus.reset();
  }

  private onEvent(event: Event) {
    switch (event.event) {
      case 'meters':
        meterBus.latest = event.data;
        meterBus.receivedAt = performance.now();
        break;
      case 'mic.changed':
        this.applyMic(event.data);
        break;
      case 'calibration.changed':
        this.calibration = event.data;
        break;
      case 'ab.changed':
        this.ab = event.data;
        break;
      case 'presets.changed':
        this.presets = event.data;
        break;
      case 'devices.changed':
        this.devices = event.data;
        break;
      case 'devices.meter':
        deviceMeterBus.peaks.set(event.data.name, event.data.peak_dbfs);
        break;
      case 'mixer.changed':
        this.applyMixer(event.data);
        break;
      case 'prefs.changed':
        this.prefs = event.data;
        break;
      case 'hotkeys.changed':
        this.hotkeys = event.data;
        break;
      case 'mixer.meters':
        mixerMeterBus.update(event.data);
        break;
      case 'mic.noise_fallback':
        this.toast(t('mic.toastNoiseFallback'), { message: t('mic.toastNoiseFallbackBody'), tone: 'danger' });
        break;
    }
  }

  /** While our own edits are pending, their settings are newer than any echo. */
  private applyMic(state: MicState) {
    const previous = this.mic;
    if (this.inFlight || this.queued) {
      if (previous) this.mic = { ...state, settings: previous.settings };
    } else {
      this.mic = state;
    }
    // The daemon replaced an unreadable config with defaults; say so once per backup.
    const recovery = state.config_recovery;
    if (recovery && recovery.backup !== this.recoveryShown) {
      this.recoveryShown = recovery.backup;
      this.toast(t('errors.configRecovered'), {
        message: t('errors.configRecoveredBody', { backup: recovery.backup }),
        tone: 'danger',
      });
    }
    if (previous && previous.input_connected !== state.input_connected && state.device) {
      this.toast(state.input_connected ? t('mic.toastReconnected') : t('mic.toastDisconnected'), {
        tone: state.input_connected ? 'info' : 'danger',
      });
    }
  }

  // --- Mixer ------------------------------------------------------------------------------

  /** Pending fader drags are newer than any echo; keep their settings. */
  private applyMixer(state: MixerState) {
    const previous = this.mixer;
    if (previous && (this.channelInFlight.size > 0 || this.channelQueue.size > 0)) {
      state = {
        ...state,
        channels: state.channels.map((channel) => {
          if (!this.channelInFlight.has(channel.id) && !this.channelQueue.has(channel.id)) return channel;
          const local = previous.channels.find((c) => c.id === channel.id);
          return local ? { ...channel, settings: local.settings } : channel;
        }),
      };
    }
    this.mixer = state;
  }

  async setMixer(request: Promise<MixerState>): Promise<void> {
    const state = await this.run(request);
    if (state) this.applyMixer(state);
  }

  /** Optimistic channel edit; rapid changes of one channel coalesce into one request. */
  editChannel(id: string, change: (settings: ChannelSettings) => void): void {
    const mixer = this.mixer;
    const channel = mixer?.channels.find((c) => c.id === id);
    if (!mixer || !channel) return;
    const settings = structuredClone(channel.settings);
    change(settings);
    this.mixer = {
      ...mixer,
      channels: mixer.channels.map((c) => (c.id === id ? { ...c, settings } : c)),
    };
    this.sendChannel(id, settings);
  }

  private sendChannel(id: string, settings: ChannelSettings) {
    if (this.channelInFlight.has(id)) {
      this.channelQueue.set(id, settings);
      return;
    }
    this.channelInFlight.add(id);
    this.client
      .call('channel.set', { id, settings })
      .then((state) => {
        this.channelInFlight.delete(id);
        if (!this.channelQueue.has(id)) this.applyMixer(state);
      })
      .catch((error) => {
        this.channelInFlight.delete(id);
        this.fail(error);
        void this.client.call('mixer.get').then((state) => this.applyMixer(state));
      })
      .finally(() => {
        const next = this.channelQueue.get(id);
        this.channelQueue.delete(id);
        if (next) this.sendChannel(id, next);
      });
  }

  /** Master fader changes coalesce like channel edits: the latest value wins. */
  private masterPending: { volume_pct?: number; muted?: boolean } | null = null;
  private masterInFlight = false;

  setMaster(change: { volume_pct?: number; muted?: boolean }): void {
    const mixer = this.mixer;
    if (!mixer) return;
    this.mixer = {
      ...mixer,
      master_volume_pct: change.volume_pct ?? mixer.master_volume_pct,
      master_muted: change.muted ?? mixer.master_muted,
    };
    if (this.masterInFlight) {
      this.masterPending = { ...this.masterPending, ...change };
      return;
    }
    this.masterInFlight = true;
    this.client
      .call('mixer.set_master', { volume_pct: change.volume_pct ?? null, muted: change.muted ?? null })
      .catch((error) => this.fail(error))
      .finally(() => {
        this.masterInFlight = false;
        const next = this.masterPending;
        this.masterPending = null;
        if (next) this.setMaster(next);
      });
  }

  /** The image for an app icon name; asks the daemon once and is `null` until it answers. */
  appIcon(name: string | null): string | null {
    if (!name) return null;
    const known = this.icons.get(name);
    if (known !== undefined) return known;
    if (this.connected && !this.iconsPending.has(name)) {
      this.iconsPending.add(name);
      this.client
        .call('apps.icon', { icon: name })
        .then((url) => this.icons.set(name, url))
        .catch(() => this.icons.set(name, null))
        .finally(() => this.iconsPending.delete(name));
    }
    return null;
  }

  // --- Chain edits --------------------------------------------------------------------------

  /** Optimistically change the chain and send it; rapid changes coalesce into one request. */
  edit(change: (settings: ChainSettings) => void): void {
    const mic = this.mic;
    if (!mic) return;
    const draft = structuredClone(mic.settings);
    change(draft);
    this.recordUndo(mic.settings);
    this.mic = { ...mic, settings: draft };
    this.send(draft);
  }

  private recordUndo(before: ChainSettings) {
    const now = performance.now();
    if (now - this.lastEditAt > UNDO_GESTURE_MS) {
      this.undoStack.push(structuredClone(before));
      if (this.undoStack.length > UNDO_DEPTH) this.undoStack.shift();
      this.redoStack = [];
    }
    this.lastEditAt = now;
    this.syncHistory();
  }

  private syncHistory() {
    this.canUndo = this.undoStack.length > 0;
    this.canRedo = this.redoStack.length > 0;
  }

  /** DSP-008: undo and redo restore the chain but keep the current mute and bypass. */
  undo(): void {
    this.travel(this.undoStack, this.redoStack);
  }

  redo(): void {
    this.travel(this.redoStack, this.undoStack);
  }

  private travel(from: ChainSettings[], to: ChainSettings[]) {
    const mic = this.mic;
    const target = from.pop();
    if (!mic || !target) return;
    to.push(structuredClone(mic.settings));
    const settings = { ...target, muted: mic.settings.muted, bypass: mic.settings.bypass };
    this.lastEditAt = 0;
    this.mic = { ...mic, settings };
    this.send(settings);
    this.syncHistory();
  }

  private send(settings: ChainSettings) {
    if (this.inFlight) {
      this.queued = settings;
      return;
    }
    this.inFlight = true;
    this.client
      .call('mic.set', { settings })
      .then((state) => {
        if (!this.queued) this.mic = state;
      })
      .catch((error) => {
        this.fail(error);
        // Resynchronise with the daemon after a rejected change.
        void this.client.call('mic.get').then((state) => (this.mic = state));
      })
      .finally(() => {
        this.inFlight = false;
        const next = this.queued;
        this.queued = null;
        if (next) this.send(next);
      });
  }

  // --- Discrete actions ---------------------------------------------------------------------

  async run<T>(request: Promise<T>): Promise<T | undefined> {
    try {
      return await request;
    } catch (error) {
      this.fail(error);
      return undefined;
    }
  }

  async setMic(request: Promise<MicState>): Promise<void> {
    const state = await this.run(request);
    if (state) this.mic = state;
  }

  async selectPreset(id: string): Promise<void> {
    if (this.mic) this.recordUndo(this.mic.settings);
    this.lastEditAt = 0;
    await this.setMic(this.client.call('mic.set_preset', { preset: id }));
  }

  // --- Onboarding ---------------------------------------------------------------------------

  startOnboarding(): void {
    this.onboarding = true;
  }

  finishOnboarding(): void {
    this.onboarding = false;
    localStorage.setItem(ONBOARDING_KEY, 'done');
  }

  // --- Toasts -------------------------------------------------------------------------------

  toast(title: string, options: { message?: string; tone?: Toast['tone'] } = {}): void {
    const id = this.nextToast++;
    this.toasts.push({ id, title, message: options.message, tone: options.tone ?? 'info' });
    setTimeout(() => this.dismiss(id), 5000);
  }

  dismiss(id: number): void {
    const index = this.toasts.findIndex((toast) => toast.id === id);
    if (index >= 0) this.toasts.splice(index, 1);
  }

  fail(error: unknown): void {
    if (error instanceof DaemonError) {
      const message = error.code === 'already_exists' ? t('errors.alreadyExists') : error.message;
      this.toast(t('errors.generic'), { message, tone: 'danger' });
    } else {
      this.toast(t('errors.generic'), { message: String(error), tone: 'danger' });
    }
  }
}
